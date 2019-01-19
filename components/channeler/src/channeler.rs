use std::cmp::Ordering;
use std::marker::Unpin;
use std::collections::HashMap;

use futures::{future, FutureExt, TryFutureExt, 
    stream, Stream, StreamExt, Sink, SinkExt};
use futures::task::{Spawn, SpawnExt};
use futures::channel::mpsc;

use proto::funder::messages::{FunderToChanneler, ChannelerToFunder, ChannelerUpdateFriend};
use common::conn::{Listener, FutTransform};
use crypto::identity::{PublicKey, compare_public_key};

use crate::types::RawConn;
use crate::overwrite_channel::overwrite_send_all;
use crate::connect_pool::{ConnectPoolControl, CpConfigClient, CpConnectClient};
use crate::listen_pool::LpConfig;


#[derive(Debug)]
pub enum ChannelerEvent<B> {
    FromFunder(FunderToChanneler<Vec<B>>),
    Connection((PublicKey, RawConn)),
    FriendEvent(FriendEvent),
    ListenerClosed,
    ConnectorClosed,
    FunderClosed,
}

#[derive(Debug)]
pub enum FriendEvent {
    IncomingMessage((PublicKey, Vec<u8>)),
    ReceiverClosed(PublicKey),
}

#[derive(Debug)]
pub enum ChannelerError {
    SpawnError,
    SendToFunderFailed,
    AddressSendFailed,
    SendConnectionEstablishedFailed,
    SendAccessControlFailed,
    ListenerConfigError,
    ListenerClosed,
    FunderClosed,
    ConnectorConfigError,
    ConnectorClosed,
}


struct Connected<T> {
    opt_sender: Option<mpsc::Sender<T>>,
}

impl<T> Connected<T> {
    pub fn new(sender: mpsc::Sender<T>) -> Self {
        Connected {
            opt_sender: Some(sender),
        }
    }

    /// Send an item. 
    /// If a failure occurs, the internal sender is removed
    /// and subsequent sends will fail too.
    ///
    /// Return value of true means send was successful.
    pub async fn send(&mut self, t: T) -> bool {
        match self.opt_sender.take() {
            Some(mut sender) => {
                match await!(sender.send(t)) {
                    Ok(()) => {
                        self.opt_sender = Some(sender);
                        true
                    },
                    Err(_t) => false,
                }
            },
            None => false,
        }
    }
}

type FriendConnected = Connected<Vec<u8>>;


enum InFriend {
    Listening,
    Connected(FriendConnected),
}

enum OutFriendStatus {
    Connecting,
    Connected(FriendConnected),
}

struct OutFriend<B> {
    config_client: CpConfigClient<B>,
    connect_client: CpConnectClient,
    status: OutFriendStatus,
}

struct Friends<B> {
    /// Friends that should connect to us:
    in_friends: HashMap<PublicKey, InFriend>,
    /// Friends that wait for our connection:
    out_friends: HashMap<PublicKey, OutFriend<B>>,
}


impl<B> Friends<B> {
    pub fn new() -> Self {
        Friends {
            in_friends: HashMap::new(),
            out_friends: HashMap::new(),
        }
    }

    /// Obtain (if possible) a FriendConnected struct corresponding to the given
    /// public_key. A FriendConnected struct allows sending messages to the remote friend.
    pub fn get_friend_connected(&mut self, public_key: &PublicKey) -> Option<&mut FriendConnected> {
        match self.in_friends.get_mut(public_key) {
            Some(in_friend) => {
                match in_friend {
                    InFriend::Listening => {},
                    InFriend::Connected(friend_connected) => return Some(friend_connected),
                }
            }
            None => {},
        };

        match self.out_friends.get_mut(public_key) {
            Some(out_friend) => {
                match &mut out_friend.status {
                    OutFriendStatus::Connecting => {},
                    OutFriendStatus::Connected(friend_connected) => return Some(friend_connected),
                }
            }
            None => {},
        };

        None
    }
}


struct Channeler<B,C,S,TF> {
    local_public_key: PublicKey,
    friends: Friends<B>,
    connector: C,
    /// Configuration sender for the listening task:
    listen_config: mpsc::Sender<LpConfig<B>>,
    spawner: S,
    to_funder: TF,
    event_sender: mpsc::Sender<ChannelerEvent<B>>,
}

impl<B,C,S,TF> Channeler<B,C,S,TF> 
where
    B: Clone + Send + Sync + 'static,
    C: FutTransform<Input=PublicKey, Output=ConnectPoolControl<B>> + Clone + Send + Sync + 'static,
    S: Spawn + Clone + Send + Sync + 'static,
    TF: Sink<SinkItem=ChannelerToFunder> + Send + Unpin,
{
    fn new(local_public_key: PublicKey,
           connector: C, 
           listen_config: mpsc::Sender<LpConfig<B>>,
           spawner: S,
           to_funder: TF,
           event_sender: mpsc::Sender<ChannelerEvent<B>>) -> Self {

        Channeler { 
            local_public_key,
            friends: Friends::new(),
            connector,
            listen_config,
            spawner,
            to_funder,
            event_sender,
        }
    }

    /// Should we wait for a connection from `friend_public_key`.
    /// In other words: Is the remote side active?
    fn is_listen_friend(&self, friend_public_key: &PublicKey) -> bool {
        compare_public_key(&self.local_public_key, friend_public_key) == Ordering::Less
    }

    fn connect_out_friend(&self, friend_public_key: &PublicKey) 
        -> Result<(), ChannelerError> {

        let out_friend = match self.friends.out_friends.get(friend_public_key) {
            Some(out_friend) => out_friend,
            None => unreachable!(), // We assert that the out_friend exists.
        };

        let mut c_connect_client = out_friend.connect_client.clone();
        let c_friend_public_key = friend_public_key.clone();
        let mut c_event_sender = self.event_sender.clone();
        let connect_fut = async move {
            let event = match await!(c_connect_client.connect()) {
                Ok(raw_conn) =>
                    ChannelerEvent::Connection((c_friend_public_key, raw_conn)),
                Err(_) => 
                    // This should only happen if there was a real problem
                    // with the connector.
                    ChannelerEvent::ConnectorClosed,
            };
            let _ = await!(c_event_sender.send(event));
        };

        self.spawner.clone().spawn(connect_fut)
            .map_err(|_| ChannelerError::SpawnError)?;

        Ok(())
    }

    /// Add friend if does not yet exist
    async fn try_create_friend<'a>(&'a mut self, friend_public_key: &'a PublicKey) 
        -> Result<(), ChannelerError> {

        if self.friends.in_friends.contains_key(friend_public_key) ||
           self.friends.out_friends.contains_key(friend_public_key) {
            // Friend already exists:
            return Ok(());
        }

        // We should add a new friend:
        if self.is_listen_friend(friend_public_key) {
            self.friends.in_friends.insert(friend_public_key.clone(), InFriend::Listening);
        } else {
            let (config_client, connect_client) = await!(self.connector.transform(friend_public_key.clone()));
            let out_friend = OutFriend {
                config_client,
                connect_client,
                status: OutFriendStatus::Connecting,
            };
            self.friends.out_friends.insert(friend_public_key.clone(), out_friend);
            self.connect_out_friend(friend_public_key)?;
        }
        Ok(())
    }

    async fn handle_from_funder(&mut self, funder_to_channeler: FunderToChanneler<Vec<B>>) 
        -> Result<(), ChannelerError>  {

        match funder_to_channeler {
            FunderToChanneler::Message((public_key, message)) => {
                let friend_connected = match self.friends.get_friend_connected(&public_key) {
                    Some(friend_connected) => friend_connected,
                    None => {
                        error!("Attempt to send a message to unavailable friend: {:?}", public_key);
                        return Ok(());
                    },
                };

                // TODO: Should we check errors here?
                let _ = await!(friend_connected.send(message));
                Ok(())
            },
            FunderToChanneler::SetAddress(opt_address) => {
                // Our local listening addresses were set.
                // We update the listener accordingly:
                
                let addresses = match opt_address {
                    Some(addresses) => addresses,
                    None => Vec::new(),
                };

                await!(self.listen_config.send(LpConfig::SetLocalAddresses(addresses)))
                    .map_err(|_| ChannelerError::ListenerConfigError)?;

                Ok(())
            },
            FunderToChanneler::UpdateFriend(channeler_update_friend) => {
                let ChannelerUpdateFriend {
                    friend_public_key,
                    friend_address,
                    local_addresses
                } = channeler_update_friend;

                await!(self.try_create_friend(&friend_public_key))?;

                if let Some(_in_friend) = self.friends.in_friends.get(&friend_public_key) {
                    // Move from a vector of vectors to a flat vector of addresses:
                    let mut total_local_addresses = Vec::new();
                    for mut addresses in local_addresses {
                        total_local_addresses.append(&mut addresses);
                    }

                    let lp_config = LpConfig::UpdateFriend((friend_public_key.clone(),
                                                            total_local_addresses));
                    await!(self.listen_config.send(lp_config))
                        .map_err(|_| ChannelerError::ListenerConfigError)?;
                } else if let Some(out_friend) = self.friends.out_friends.get_mut(&friend_public_key) {
                    await!(out_friend.config_client.config(friend_address))
                        .map_err(|_| ChannelerError::ConnectorConfigError)?;
                }

                Ok(())
            },
            FunderToChanneler::RemoveFriend(friend_public_key) => {
                if let Some(_) = self.friends.in_friends.remove(&friend_public_key) {
                    let lp_config = LpConfig::RemoveFriend(friend_public_key.clone());
                    await!(self.listen_config.send(lp_config))
                        .map_err(|_| ChannelerError::ListenerConfigError)?;
                    return Ok(());
                }

                self.friends.out_friends.remove(&friend_public_key);

                Ok(())
            }
        }
    }

    async fn handle_connection(&mut self, 
                               friend_public_key: PublicKey, 
                               raw_conn: RawConn)
        -> Result<(), ChannelerError>  {

        let (sender, receiver) = raw_conn;
        // We use an overwrite channel to make sure we are never stuck on trying to send a
        // message to remote friend. A friend only needs to know the most recent message,
        // so previous pending messages may be discarded.
        let (friend_sender, friend_receiver) = mpsc::channel(0);
        self.spawner.spawn(overwrite_send_all(sender, friend_receiver)
                      .map_err(|e| error!("overwrite_send_all() error: {:?}", e))
                      .map(|_| ()))
            .map_err(|_| ChannelerError::SpawnError)?;


        if let Some(in_friend) = self.friends.in_friends.get_mut(&friend_public_key) {
            match in_friend {
                InFriend::Connected(_) => {
                    warn!("Already connected to in_friend: {:?}. Aborting.", 
                          friend_public_key);
                    return Ok(());
                },
                InFriend::Listening => {
                    *in_friend = InFriend::Connected(Connected::new(friend_sender))
                }
            }

        } else if let Some(mut out_friend) = self.friends.out_friends.get_mut(&friend_public_key) {
            match out_friend.status {
                OutFriendStatus::Connected(_) => {
                    warn!("Already connected to out_friend: {:?}. Aborting.", 
                          friend_public_key);
                    return Ok(());
                },
                OutFriendStatus::Connecting => {
                    out_friend.status = OutFriendStatus::Connected(Connected::new(friend_sender))
                }
            }
        }

        let mut c_event_sender = self.event_sender.clone();
        let c_friend_public_key = friend_public_key.clone();
        let mut receiver = receiver
            .map(move |data|
                ChannelerEvent::FriendEvent(FriendEvent::IncomingMessage((c_friend_public_key.clone(), data))));
        let c_friend_public_key = friend_public_key.clone();
        let fut_recv = async move {
            let _ = await!(c_event_sender.send_all(&mut receiver));
            let receiver_closed_event = ChannelerEvent::FriendEvent(
                FriendEvent::ReceiverClosed(c_friend_public_key.clone()));
            let _ = await!(c_event_sender.send(receiver_closed_event));

        };

        self.spawner.spawn(fut_recv)
            .map_err(|_| ChannelerError::SpawnError)?;

        // Report to Funder that the friend is online:
        let to_funder = ChannelerToFunder::Online(friend_public_key.clone());
        await!(self.to_funder.send(to_funder))
            .map_err(|_| ChannelerError::SendToFunderFailed)?;

        Ok(())
    }

    async fn handle_friend_event(&mut self, friend_event: FriendEvent)
        -> Result<(), ChannelerError>  {

        match friend_event {
            FriendEvent::IncomingMessage((friend_public_key, data)) => {
                let message = ChannelerToFunder::Message((friend_public_key, data));
                await!(self.to_funder.send(message))
                    .map_err(|_| ChannelerError::SendToFunderFailed)?
            },
            FriendEvent::ReceiverClosed(friend_public_key) => {
                if self.friends.get_friend_connected(&friend_public_key).is_some() {
                    // Report Funder that the friend is offline:
                    let to_funder = ChannelerToFunder::Offline(friend_public_key.clone());
                    await!(self.to_funder.send(to_funder))
                        .map_err(|_| ChannelerError::SendToFunderFailed)?;
                }

                if let Some(in_friend) = self.friends.in_friends.get_mut(&friend_public_key) {
                    *in_friend = InFriend::Listening;

                } else if let Some(out_friend) = self.friends.out_friends.get_mut(&friend_public_key) {
                    // Request a new connection
                    out_friend.status = OutFriendStatus::Connecting;
                    self.connect_out_friend(&friend_public_key)?;
                }
            },
        }
        Ok(())
    }
}


#[allow(unused)]
async fn channeler_loop<FF,TF,B,C,L,S>(
                        local_public_key: PublicKey,
                        from_funder: FF, 
                        to_funder: TF,
                        connector: C,
                        listener: L,
                        spawner: S) -> Result<(), ChannelerError>
where
    FF: Stream<Item=FunderToChanneler<Vec<B>>> + Unpin,
    TF: Sink<SinkItem=ChannelerToFunder> + Send + Unpin,
    B: Clone + Send + Sync + 'static + std::fmt::Debug,
    C: FutTransform<Input=PublicKey, Output=ConnectPoolControl<B>> + Clone + Send + Sync + 'static,
    L: Listener<Connection=(PublicKey, RawConn), Config=LpConfig<B>, Arg=()> + Clone + Send,
    S: Spawn + Clone + Send + Sync + 'static,
{

    let (event_sender, event_receiver) = mpsc::channel(0);

    let (listen_config, incoming_listen_conns) = listener.listen(());

    let mut channeler = Channeler::new(local_public_key, 
                                       connector, 
                                       listen_config,
                                       spawner,
                                       to_funder,
                                       event_sender);

    // Forward incoming listen connections:
    let mut c_event_sender = channeler.event_sender.clone();
    let mut incoming_listen_conns = incoming_listen_conns
        .map(|pk_conn| ChannelerEvent::Connection(pk_conn));
    let send_listen_conns_fut = async move {
        let _ = await!(c_event_sender.send_all(&mut incoming_listen_conns));
        // If we reach here it means an error occured.
        let _ = await!(c_event_sender.send(ChannelerEvent::ListenerClosed));
    };
    channeler.spawner.spawn(send_listen_conns_fut)
        .map_err(|_| ChannelerError::SpawnError)?;

    let from_funder = from_funder
        .map(|funder_to_channeler| ChannelerEvent::FromFunder(funder_to_channeler))
        .chain(stream::once(future::ready(ChannelerEvent::FunderClosed)));

    let mut events = event_receiver.select(from_funder);

    while let Some(event) = await!(events.next()) {
        match event {
            ChannelerEvent::FromFunder(funder_to_channeler) => 
                await!(channeler.handle_from_funder(funder_to_channeler))?,
            ChannelerEvent::Connection((public_key, raw_conn)) =>
                await!(channeler.handle_connection(public_key, raw_conn))?,
            ChannelerEvent::FriendEvent(friend_event) =>
                await!(channeler.handle_friend_event(friend_event))?,
            ChannelerEvent::ListenerClosed => return Err(ChannelerError::ListenerClosed),
            ChannelerEvent::ConnectorClosed => return Err(ChannelerError::ConnectorClosed),
            ChannelerEvent::FunderClosed => return Err(ChannelerError::FunderClosed),
        };
    }
    Ok(())
}

/*

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::ThreadPool;

    use common::dummy_connector::DummyConnector;
    use common::dummy_listener::DummyListener;
    use crypto::identity::{PublicKey, PUBLIC_KEY_LEN};

    /// Test the case of a friend the channeler initiates connection to.
    async fn task_channeler_loop_connect_friend<S>(mut spawner: S)
    where
        S: Spawn + Clone + Send + Sync + 'static,
    {

        let (mut funder_sender, from_funder) = mpsc::channel(0);
        let (to_funder, mut funder_receiver) = mpsc::channel(0);

        // We sort the public keys ahead of time, so that we know how to break ties.
        // Our local public key will be pks[1]. pks[0] < pks[1] < pks[2]
        //
        // pks[1] >= pks[0], so pks[0] be an active send friend (We initiate connection)
        // pks[1] < pks[2], hence pks[2] will be a listen friend. (We wait for him to connect)
        let mut pks = (0 .. 3)
            .map(|i| PublicKey::from(&[i; PUBLIC_KEY_LEN]))
            .collect::<Vec<PublicKey>>();
        pks.sort_by(compare_public_key);


        let (conn_request_sender, mut conn_request_receiver) = mpsc::channel(0);
        let connector = DummyConnector::new(conn_request_sender);

        let (listener_req_sender, mut listener_req_receiver) = mpsc::channel(0);
        let listener = DummyListener::new(listener_req_sender, spawner.clone());


        spawner.spawn(channeler_loop(pks[1].clone(),
                       from_funder,
                       to_funder,
                       connector,
                       listener,
                       spawner.clone())
            .map_err(|e| error!("Error in channeler_loop(): {:?}", e))
            .map(|_| ())).unwrap();

        // Play with changing relay addresses:
        await!(funder_sender.send(FunderToChanneler::SetAddress(Some(0x1337u32)))).unwrap();
        let listener_request = await!(listener_req_receiver.next()).unwrap();
        let (ref address, _) = listener_request.arg;
        assert_eq!(address, &0x1337u32);
        // Empty relay address:
        await!(funder_sender.send(FunderToChanneler::SetAddress(None))).unwrap();

        // This is the final address we set for our relay:
        await!(funder_sender.send(FunderToChanneler::SetAddress(Some(0x1u32)))).unwrap();
        let listener_request = await!(listener_req_receiver.next()).unwrap();
        let (ref address, _) = listener_request.arg;
        assert_eq!(address, &0x1u32);

        // Add a friend:
        await!(funder_sender.send(FunderToChanneler::AddFriend((pks[0].clone(), 0x0u32)))).unwrap();
        let conn_request = await!(conn_request_receiver.next()).unwrap();
        assert_eq!(conn_request.address, (0x0u32, pks[0].clone()));

        let (mut pk0_sender, remote_receiver) = mpsc::channel(0);
        let (remote_sender, mut pk0_receiver) = mpsc::channel(0);
        conn_request.reply((remote_sender, remote_receiver));

        // Friend should be reported as online:
        let channeler_to_funder = await!(funder_receiver.next()).unwrap();
        match channeler_to_funder {
            ChannelerToFunder::Online(public_key) => assert_eq!(public_key, pks[0]),
            _ => unreachable!(),
        };

        // Send a message to pks[0]:
        await!(funder_sender.send(FunderToChanneler::Message((pks[0].clone(), vec![1,2,3])))).unwrap();
        assert_eq!(await!(pk0_receiver.next()).unwrap(), vec![1,2,3]);

        // Send a message from pks[0]:
        await!(pk0_sender.send(vec![3,2,1])).unwrap();

        // We expect to get the message from pks[0]:
        let channeler_to_funder = await!(funder_receiver.next()).unwrap();
        match channeler_to_funder {
            ChannelerToFunder::Message((public_key, message)) => {
                assert_eq!(public_key, pks[0]);
                assert_eq!(message, vec![3,2,1]);
            },
            _ => unreachable!(),
        };

        // Drop pks[0] connection:
        drop(pk0_sender);
        drop(pk0_receiver);

        // pks[0] should be reported as offline:
        let channeler_to_funder = await!(funder_receiver.next()).unwrap();
        match channeler_to_funder {
            ChannelerToFunder::Offline(public_key) => assert_eq!(public_key, pks[0]),
            _ => unreachable!(),
        };

        // Connection to pks[0] should be attempted again:
        let conn_request = await!(conn_request_receiver.next()).unwrap();
        assert_eq!(conn_request.address, (0x0u32, pks[0].clone()));

        let (pk0_sender, remote_receiver) = mpsc::channel(0);
        let (remote_sender, pk0_receiver) = mpsc::channel(0);
        conn_request.reply((remote_sender, remote_receiver));

        // Online report:
        let channeler_to_funder = await!(funder_receiver.next()).unwrap();
        match channeler_to_funder {
            ChannelerToFunder::Online(public_key) => assert_eq!(public_key, pks[0]),
            _ => unreachable!(),
        };

        // Drop pks[0] connection:
        drop(pk0_sender);
        drop(pk0_receiver);

        // Offline report:
        let channeler_to_funder = await!(funder_receiver.next()).unwrap();
        match channeler_to_funder {
            ChannelerToFunder::Offline(public_key) => assert_eq!(public_key, pks[0]),
            _ => unreachable!(),
        };

        // Remove friend:
        await!(funder_sender.send(FunderToChanneler::RemoveFriend(pks[0].clone()))).unwrap();
    }

    #[test]
    fn test_channeler_loop_connect_friend() {
        let mut thread_pool = ThreadPool::new().unwrap();
        thread_pool.run(task_channeler_loop_connect_friend(thread_pool.clone()));
    }

    /// Test the case of the channeler waiting for a connection from a friend.
    async fn task_channeler_loop_listen_friend<S>(mut spawner: S)
    where
        S: Spawn + Clone + Send + Sync + 'static,
    {

        let (mut funder_sender, from_funder) = mpsc::channel(0);
        let (to_funder, mut funder_receiver) = mpsc::channel(0);

        // We sort the public keys ahead of time, so that we know how to break ties.
        // Our local public key will be pks[1]. pks[0] < pks[1] < pks[2]
        //
        // pks[1] >= pks[0], so pks[0] be an active send friend (We initiate connection)
        // pks[1] < pks[2], hence pks[2] will be a listen friend. (We wait for him to connect)
        let mut pks = (0 .. 3)
            .map(|i| PublicKey::from(&[i; PUBLIC_KEY_LEN]))
            .collect::<Vec<PublicKey>>();
        pks.sort_by(compare_public_key);


        let (conn_request_sender, _conn_request_receiver) = mpsc::channel(0);
        let connector = DummyConnector::new(conn_request_sender);

        let (listener_req_sender, mut listener_req_receiver) = mpsc::channel(0);
        let listener = DummyListener::new(listener_req_sender, spawner.clone());


        spawner.spawn(channeler_loop(pks[1].clone(),
                       from_funder,
                       to_funder,
                       connector,
                       listener,
                       spawner.clone())
            .map_err(|e| error!("Error in channeler_loop(): {:?}", e))
            .map(|_| ())).unwrap();

        // Set address for our relay:
        await!(funder_sender.send(FunderToChanneler::SetAddress(Some(0x1u32)))).unwrap();
        let mut listener_request = await!(listener_req_receiver.next()).unwrap();
        let (ref address, _) = listener_request.arg;
        assert_eq!(address, &0x1u32);

        // Add a friend:
        await!(funder_sender.send(FunderToChanneler::AddFriend((pks[2].clone(), 0x2u32)))).unwrap();

        let access_control_op = await!(listener_request.config_receiver.next()).unwrap();
        assert_eq!(access_control_op, AccessControlOp::Add(pks[2].clone()));

        // Set up connection, exchange messages and close the connection a few times:
        for _ in 0 .. 3 {
            // The channeler now listens. It waits for an incoming connection from pks[2]
            // Set up a connection from pks[2]:
            let (mut pk2_sender, receiver) = mpsc::channel(0);
            let (sender, mut pk2_receiver) = mpsc::channel(0);
            await!(listener_request.conn_sender.send((pks[2].clone(), (sender, receiver)))).unwrap();

            // Friend should be reported as online:
            let channeler_to_funder = await!(funder_receiver.next()).unwrap();
            match channeler_to_funder {
                ChannelerToFunder::Online(public_key) => assert_eq!(public_key, pks[2]),
                _ => unreachable!(),
            };

            // Send a message to pks[2]:
            await!(funder_sender.send(FunderToChanneler::Message((pks[2].clone(), vec![1,2,3])))).unwrap();
            assert_eq!(await!(pk2_receiver.next()).unwrap(), vec![1,2,3]);

            // Send a message from pks2:
            await!(pk2_sender.send(vec![3,2,1])).unwrap();

            // We expect to get the message from pks[2]:
            let channeler_to_funder = await!(funder_receiver.next()).unwrap();
            match channeler_to_funder {
                ChannelerToFunder::Message((public_key, message)) => {
                    assert_eq!(public_key, pks[2]);
                    assert_eq!(message, vec![3,2,1]);
                },
                _ => unreachable!(),
            };

            // Drop pks[2] connection:
            drop(pk2_sender);
            drop(pk2_receiver);

            // Friend should be reported as offline:
            let channeler_to_funder = await!(funder_receiver.next()).unwrap();
            match channeler_to_funder {
                ChannelerToFunder::Offline(public_key) => assert_eq!(public_key, pks[2]),
                _ => unreachable!(),
            };
        }

        // Remove friend:
        await!(funder_sender.send(FunderToChanneler::RemoveFriend(pks[2].clone()))).unwrap();
    }

    #[test]
    fn test_channeler_loop_listen_friend() {
        let mut thread_pool = ThreadPool::new().unwrap();
        thread_pool.run(task_channeler_loop_listen_friend(thread_pool.clone()));
    }


    // TODO: Add tests to make sure access control works properly?
    // If a friend with a strange public key tries to connect, he should not be able to succeed?
}
*/
