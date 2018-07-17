use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;

use num_bigint::BigUint;
use num_traits::identities::Zero;

use crypto::identity::PublicKey;
use crypto::rand_values::RandValue;

use proto::networker::ChannelToken;

use super::types::{NeighborTcOp, NeighborMoveToken, RequestSendMessage};
use super::super::messages::{NeighborStatus};

use super::token_channel::directional::{DirectionalTokenChannel, 
    ReceiveMoveTokenOutput, ReceiveMoveTokenError, TokenChannelSender};
use super::token_channel::types::NeighborMoveTokenInner;

use app_manager::messages::{SetNeighborRemoteMaxDebt, SetNeighborMaxChannels, 
    AddNeighbor, RemoveNeighbor, ResetNeighborChannel, SetNeighborStatus};


#[allow(dead_code)]
pub enum TokenChannelStatus {
    Valid,
    /// Inconsistent means that the remote side showed disagreement about the 
    /// token channel, and this channel is waiting for a local human intervention.
    Inconsistent {
        current_token: ChannelToken,
        balance_for_reset: i64,
    },
}

#[allow(unused)]
pub struct TokenChannelSlot {
    pub tc_state: DirectionalTokenChannel,
    pub tc_status: TokenChannelStatus,
    pub wanted_remote_max_debt: u64,
    pub pending_operations: VecDeque<NeighborTcOp>,
    // Pending operations to be sent to the token channel.
}


#[allow(unused)]
impl TokenChannelSlot {
    pub fn new(local_public_key: &PublicKey,
               remote_public_key: &PublicKey,
               token_channel_index: u16) -> TokenChannelSlot {
        TokenChannelSlot {
            tc_state: DirectionalTokenChannel::new(local_public_key,
                                           remote_public_key,
                                           token_channel_index),
            tc_status: TokenChannelStatus::Valid,
            wanted_remote_max_debt: 0,
            pending_operations: VecDeque::new(),
        }
    }

    pub fn new_from_reset(local_public_key: &PublicKey,
                           remote_public_key: &PublicKey,
                           token_channel_index: u16,
                           current_token: &ChannelToken,
                           balance: i64) -> TokenChannelSlot {

        TokenChannelSlot {
            tc_state: DirectionalTokenChannel::new_from_reset(local_public_key,
                                                      remote_public_key,
                                                      token_channel_index,
                                                      current_token,
                                                      balance),
            tc_status: TokenChannelStatus::Valid,
            wanted_remote_max_debt: 0,
            pending_operations: VecDeque::new(),
        }
    }
}

#[allow(unused)]
pub struct NeighborState {
    neighbor_socket_addr: Option<SocketAddr>, 
    pub local_max_channels: u16,
    pub remote_max_channels: u16,
    pub status: NeighborStatus,
    // Enabled or disabled?
    pub token_channel_slots: HashMap<u16, TokenChannelSlot>,
    pending_requests: VecDeque<RequestSendMessage>,
    // Pending operations that could be sent through any token channel.
    ticks_since_last_incoming: usize,
    // Number of time ticks since last incoming message
    ticks_since_last_outgoing: usize,
    // Number of time ticks since last outgoing message
    
    // TODO: Keep state of payment requests to Funder
    
    // TODO: Keep state of requests to database? Only write to RAM after getting acknowledgement
    // from database.
}

#[allow(unused)]
impl NeighborState {
    pub fn new(neighbor_socket_addr: Option<SocketAddr>,
               local_max_channels: u16) -> NeighborState {

        NeighborState {
            neighbor_socket_addr,
            local_max_channels,
            remote_max_channels: local_max_channels,    
            // Initially we assume that the remote side has the same amount of channels as we do.
            status: NeighborStatus::Disable,
            token_channel_slots: HashMap::new(),
            pending_requests: VecDeque::new(),
            ticks_since_last_incoming: 0,
            ticks_since_last_outgoing: 0,
        }
    }

    /// Get the total trust we have in this neighbor.
    /// This is the total sum of all remote_max_debt for all the token channels we have with this
    /// neighbor. In other words, this is the total amount of money we can lose if this neighbor
    /// leaves and never returns.
    pub fn get_trust(&self) -> BigUint {
        let mut sum: BigUint = BigUint::zero();
        for token_channel_slot in self.token_channel_slots.values() {
            let remote_max_debt: BigUint = token_channel_slot.wanted_remote_max_debt.into();
            sum += remote_max_debt;
        }
        sum
    }
}

#[allow(unused)]
pub struct MessengerState {
    local_public_key: PublicKey,
    neighbors: HashMap<PublicKey, NeighborState>,
}

#[allow(unused)]
#[derive(Clone)]
pub struct SmInitTokenChannel {
    pub neighbor_public_key: PublicKey,
    pub channel_index: u16,
}

#[allow(unused)]
#[derive(Clone)]
pub struct SmTokenChannelPushOp {
    pub neighbor_public_key: PublicKey, 
    pub channel_index: u16, 
    pub neighbor_op: NeighborTcOp
}

#[derive(Clone)]
pub struct SmNeighborPushRequest {
    pub neighbor_public_key: PublicKey,
    pub request: RequestSendMessage,
}

#[allow(unused)]
#[derive(Clone)]
pub struct SmResetTokenChannel {
    pub neighbor_public_key: PublicKey, 
    pub channel_index: u16, 
    pub reset_token: ChannelToken,
    pub balance_for_reset: i64,
}

#[allow(unused)]
#[derive(Clone)]
// TODO: Possibly change name to SmIncomingNeighborMoveToken.
pub struct SmApplyNeighborMoveToken {
    pub neighbor_public_key: PublicKey, 
    pub neighbor_move_token: NeighborMoveToken,
}

#[allow(unused)]
#[derive(Clone)]
pub struct SmOutgoingNeighborMoveToken {
    pub neighbor_public_key: PublicKey, 
    pub neighbor_move_token: NeighborMoveToken,
}


#[allow(unused)]
#[derive(Clone)]
pub enum StateMutateMessage {
    SetNeighborRemoteMaxDebt(SetNeighborRemoteMaxDebt),
    SetNeighborMaxChannels(SetNeighborMaxChannels),
    ResetNeighborChannel(ResetNeighborChannel),
    AddNeighbor(AddNeighbor),
    RemoveNeighbor(RemoveNeighbor),
    SetNeighborStatus(SetNeighborStatus),
    InitTokenChannel(SmInitTokenChannel),
    TokenChannelPushOp(SmTokenChannelPushOp),
    NeighborPushRequest(SmNeighborPushRequest),
    ResetTokenChannel(SmResetTokenChannel),
    ApplyNeighborMoveToken(SmApplyNeighborMoveToken),
    OutgoingNeighborMoveToken(SmOutgoingNeighborMoveToken),
}


#[allow(unused)]
#[derive(Debug)]
pub enum MessengerStateError {
    NeighborDoesNotExist,
    TokenChannelDoesNotExist,
    NeighborAlreadyExists,
    TokenChannelAlreadyExists,
    ReceiveMoveTokenError(ReceiveMoveTokenError),
    TokenChannelAlreadyOutgoing,
}

#[allow(unused)]
impl MessengerState {
    pub fn new() -> MessengerState {
        // TODO: Initialize from database somehow.
        unreachable!();
    }

    /// Get total trust (in credits) we put on all the neighbors together.
    pub fn get_total_trust(&self) -> BigUint {
        let mut sum: BigUint = BigUint::zero();
        for neighbor in self.neighbors.values() {
            sum += neighbor.get_trust();
        }
        sum
    }

    pub fn get_neighbors(&self) -> &HashMap<PublicKey, NeighborState> {
        &self.neighbors
    }

    pub fn get_local_public_key(&self) -> &PublicKey {
        &self.local_public_key
    }

    pub fn set_neighbor_remote_max_debt(&mut self, 
                                        set_neighbor_remote_max_debt: SetNeighborRemoteMaxDebt)
                                        -> Result<(), MessengerStateError> {

        let neighbor_state = self.neighbors.get_mut(&set_neighbor_remote_max_debt.neighbor_public_key)
            .ok_or(MessengerStateError::NeighborDoesNotExist)?;
        
        // Find the token channel slot:
        let token_channel_slot = neighbor_state.token_channel_slots
            .get_mut(&set_neighbor_remote_max_debt.channel_index)
            .ok_or(MessengerStateError::TokenChannelDoesNotExist)?;

        token_channel_slot.wanted_remote_max_debt = set_neighbor_remote_max_debt.remote_max_debt;
        Ok(())
    }


    // TODO: This method is very similar to reset_token_channel. Could/Should we unite them?
    pub fn reset_neighbor_channel(&mut self, 
                                    reset_neighbor_channel: ResetNeighborChannel) 
                                    -> Result<(), MessengerStateError> {
                                        
        let neighbor_state = self.neighbors.get_mut(&reset_neighbor_channel.neighbor_public_key)
            .ok_or(MessengerStateError::NeighborDoesNotExist)?;

        let new_token_channel_slot = TokenChannelSlot::new_from_reset(
            &self.local_public_key,
            &reset_neighbor_channel.neighbor_public_key,
            reset_neighbor_channel.channel_index,
            &reset_neighbor_channel.current_token,
            reset_neighbor_channel.balance_for_reset);

        // Replace the old token channel slot with the new one:
        if !neighbor_state.token_channel_slots.contains_key(&reset_neighbor_channel.channel_index) {
            return Err(MessengerStateError::TokenChannelDoesNotExist);
        }
        
        neighbor_state.token_channel_slots.insert(reset_neighbor_channel.channel_index, 
                                                  new_token_channel_slot);

        Ok(())
    }

    pub fn set_neighbor_max_channels(&mut self, 
                                    set_neighbor_max_channels: SetNeighborMaxChannels) 
                                    -> Result<(), MessengerStateError> {

        // Check if we have the requested neighbor:
        let neighbor_state = self.neighbors.get_mut(&set_neighbor_max_channels.neighbor_public_key)
            .ok_or(MessengerStateError::NeighborDoesNotExist)?;

        neighbor_state.local_max_channels = set_neighbor_max_channels.max_channels;

        Ok(())
    }

    pub fn add_neighbor(&mut self, 
                        add_neighbor: AddNeighbor) 
                        -> Result<(), MessengerStateError> {

        // If we already have the neighbor: return error.
        if self.neighbors.contains_key(&add_neighbor.neighbor_public_key) {
            return Err(MessengerStateError::NeighborAlreadyExists);
        }

        // Otherwise, we add a new neighbor:
        let neighbor_state = NeighborState::new(
                add_neighbor.neighbor_socket_addr,
                add_neighbor.max_channels);

        self.neighbors.insert(add_neighbor.neighbor_public_key.clone(), neighbor_state);

        Ok(())
    }

    fn get_neighbor(&self, neighbor_public_key: &PublicKey) 
        -> Result<&NeighborState, MessengerStateError> {

        self.neighbors.get(neighbor_public_key)
            .ok_or(MessengerStateError::NeighborDoesNotExist)
    }

    pub fn remove_neighbor(&mut self, 
                        remove_neighbor: RemoveNeighbor) 
                        -> Result<(), MessengerStateError> {

        let _ = self.neighbors.remove(&remove_neighbor.neighbor_public_key)
            .ok_or(MessengerStateError::NeighborDoesNotExist)?;

        Ok(())
    }

    pub fn set_neighbor_status(&mut self, 
                        set_neighbor_status: SetNeighborStatus) 
                        -> Result<(), MessengerStateError> {

        // Check if we have the requested neighbor:
        let neighbor_state = self.neighbors.get_mut(&set_neighbor_status.neighbor_public_key)
            .ok_or(MessengerStateError::NeighborDoesNotExist)?;

        neighbor_state.status = set_neighbor_status.status;

        Ok(())
    }


    pub fn init_token_channel(&mut self, init_token_channel: SmInitTokenChannel)
        -> Result<(), MessengerStateError> {

        if self.get_neighbor(&init_token_channel.neighbor_public_key)?
            .token_channel_slots.contains_key(&init_token_channel.channel_index) {
            return Err(MessengerStateError::TokenChannelAlreadyExists);
        }

        let neighbor = self.neighbors.get_mut(&init_token_channel.neighbor_public_key)
            .ok_or(MessengerStateError::NeighborDoesNotExist)?;

        neighbor.token_channel_slots.insert(
            init_token_channel.channel_index,
            TokenChannelSlot::new(&self.local_public_key,
                                     &init_token_channel.neighbor_public_key,
                                     init_token_channel.channel_index));
        Ok(())
    }

    pub fn token_channel_push_op(&mut self, 
                                 token_channel_push_op: SmTokenChannelPushOp) 
        -> Result<(), MessengerStateError> {

        let neighbor = self.neighbors.get_mut(&token_channel_push_op.neighbor_public_key)
            .ok_or(MessengerStateError::NeighborDoesNotExist)?;

        let token_channel_slot = neighbor.token_channel_slots
            .get_mut(&token_channel_push_op.channel_index)
            .ok_or(MessengerStateError::TokenChannelDoesNotExist)?;

        token_channel_slot.pending_operations.push_back(token_channel_push_op.neighbor_op.clone());

        Ok(())
    }

    pub fn neighbor_push_request(&mut self, 
                                 neighbor_push_request: SmNeighborPushRequest) 
        -> Result<(), MessengerStateError> {

        let neighbor = self.neighbors.get_mut(&neighbor_push_request.neighbor_public_key)
            .ok_or(MessengerStateError::NeighborDoesNotExist)?;

        neighbor.pending_requests.push_back(neighbor_push_request.request);

        Ok(())
    }

    pub fn reset_token_channel(&mut self, 
                                 reset_token_channel: SmResetTokenChannel) 
        -> Result<(), MessengerStateError> {

        let neighbor = self.neighbors.get_mut(&reset_token_channel.neighbor_public_key)
            .ok_or(MessengerStateError::NeighborDoesNotExist)?;

        let token_channel_slot = TokenChannelSlot::new_from_reset(
            &self.local_public_key,
            &reset_token_channel.neighbor_public_key,
            reset_token_channel.channel_index,
            &reset_token_channel.reset_token,
            reset_token_channel.balance_for_reset);

        let _ = neighbor.token_channel_slots.insert(
            reset_token_channel.channel_index, 
            token_channel_slot);

        Ok(())
    }

    pub fn apply_neighbor_move_token(&mut self, 
                                 apply_neighbor_move_token: SmApplyNeighborMoveToken) 
        -> Result<ReceiveMoveTokenOutput, MessengerStateError> {


        let neighbor = self.neighbors.get_mut(&apply_neighbor_move_token.neighbor_public_key)
            .ok_or(MessengerStateError::NeighborDoesNotExist)?;

        let channel_index = apply_neighbor_move_token
            .neighbor_move_token
            .token_channel_index;

        let token_channel_slot = neighbor.token_channel_slots
            .get_mut(&channel_index)
            .ok_or(MessengerStateError::TokenChannelDoesNotExist)?;

        let NeighborMoveToken { operations, old_token, rand_nonce, .. } = 
            apply_neighbor_move_token.neighbor_move_token;

        let inner_move_token = NeighborMoveTokenInner {
            operations,
            old_token,
            rand_nonce,
        };

        let new_token = apply_neighbor_move_token.neighbor_move_token.new_token;

        token_channel_slot.tc_state.receive_move_token(inner_move_token, new_token)
            .map_err(MessengerStateError::ReceiveMoveTokenError)

    }

    pub fn begin_outgoing_move_token(&mut self, 
                                     neighbor_public_key: &PublicKey,
                                     channel_index: u16) 
                            -> Result<TokenChannelSender, MessengerStateError> {

        let neighbor = self.neighbors.get_mut(neighbor_public_key)
            .ok_or(MessengerStateError::NeighborDoesNotExist)?;
        let token_channel_slot = neighbor.token_channel_slots
            .get_mut(&channel_index)
            .ok_or(MessengerStateError::TokenChannelDoesNotExist)?;

        let tc_sender = token_channel_slot.tc_state.begin_outgoing_move_token()
            .ok_or(MessengerStateError::TokenChannelAlreadyOutgoing)?;

        Ok(tc_sender)
    }

    pub fn commit_outgoing_move_token(&mut self, 
                                      neighbor_public_key: &PublicKey, 
                                      channel_index: u16,
                                      tc_sender: TokenChannelSender,
                                      rand_nonce: RandValue) 
                            -> Result<NeighborMoveToken, MessengerStateError>  {

        let neighbor = self.neighbors.get_mut(neighbor_public_key)
            .ok_or(MessengerStateError::NeighborDoesNotExist)?;
        let token_channel_slot = neighbor.token_channel_slots
            .get_mut(&channel_index)
            .ok_or(MessengerStateError::TokenChannelDoesNotExist)?;

        let neighbor_move_token = token_channel_slot.tc_state
            .commit_outgoing_move_token(tc_sender, rand_nonce);

        Ok(neighbor_move_token)
    }
}
