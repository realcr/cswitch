use database::transaction::Transaction;

use crate::token_channel::TcDbClient;

use crate::router::types::{RouterControl, RouterDbClient, RouterError, RouterInfo, RouterOp};
use crate::router::{handle_config, handle_friend, handle_liveness, handle_relays, handle_route};

pub async fn handle_router_op<RC>(
    control: &mut RouterControl<'_, RC>,
    info: &RouterInfo,
    router_op: RouterOp,
) -> Result<(), RouterError>
where
    RC: RouterDbClient,
    RC::TcDbClient: Transaction + Send,
    <RC::TcDbClient as TcDbClient>::McDbClient: Send,
{
    match router_op {
        RouterOp::AddCurrency(friend_public_key, currency) => {
            handle_config::add_currency(control, info, friend_public_key, currency).await
        }
        RouterOp::RemoveCurrency(friend_public_key, currency) => todo!(),
        RouterOp::SetRemoteMaxDebt(friend_public_key, currency, remote_max_debt) => {
            handle_config::set_remote_max_debt(
                control,
                info,
                friend_public_key,
                currency,
                remote_max_debt,
            )
            .await
        }
        RouterOp::SetLocalMaxDebt(friend_public_key, currency, local_max_debt) => {
            handle_config::set_local_max_debt(
                control,
                info,
                friend_public_key,
                currency,
                local_max_debt,
            )
            .await
        }
        RouterOp::OpenCurrency(friend_public_key, currency) => {
            handle_config::open_currency(control, friend_public_key, currency).await
        }
        RouterOp::CloseCurrency(friend_public_key, currency) => {
            handle_config::close_currency(control, friend_public_key, currency).await
        }
        RouterOp::FriendMessage(friend_public_key, friend_message) => {
            handle_friend::incoming_friend_message(control, info, friend_public_key, friend_message)
                .await
        }
        RouterOp::SetFriendOnline(friend_public_key) => todo!(),
        RouterOp::SetFriendOffline(friend_public_key) => todo!(),
        RouterOp::UpdateFriendLocalRelays(friend_public_key, friend_local_relays) => todo!(),
        RouterOp::UpdateLocalRelays(local_relays) => todo!(),
        RouterOp::SendRequest(currency, mc_request) => todo!(),
        RouterOp::SendResponse(mc_response) => todo!(),
        RouterOp::SendCancel(mc_cancel) => todo!(),
    }?;
    todo!();
}
