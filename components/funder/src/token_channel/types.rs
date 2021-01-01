use std::collections::HashMap;

use futures::channel::oneshot;

use common::async_rpc::{AsyncOpResult, AsyncOpStream};
use common::u256::U256;

use proto::crypto::Signature;
use proto::funder::messages::{Currency, McBalance, MoveToken, ResetBalance, ResetTerms};

use database::interface::funder::CurrencyConfig;

use crate::mutual_credit::types::McDbClient;
use crate::types::MoveTokenHashed;

#[derive(Debug)]
pub enum TcOpError {
    SendOpFailed,
    ResponseOpFailed(oneshot::Canceled),
}

pub type TcOpResult<T> = Result<T, TcOpError>;
pub type TcOpSenderResult<T> = oneshot::Sender<TcOpResult<T>>;

/// Status of a TokenChannel. Could be either outgoing, incoming or inconsistent.
#[derive(Debug)]
pub enum TcStatus {
    ConsistentIn(MoveTokenHashed),                     // (move_token_in)
    ConsistentOut(MoveToken, Option<MoveTokenHashed>), // (move_token_out, last_move_token_in)
    // TODO: Is it too wasteful to save all the balances in memory?
    // We took this decision because we assume that we might need to send all those balances in a
    // message at some point anyways. Maybe could be improved in the future.
    Inconsistent(ResetTerms, Option<ResetTerms>), // (local_reset_terms, Option<remote_reset_terms)
}

pub trait TcDbClient {
    type McDbClient: McDbClient;
    // TODO: Maybe should return an Option instead? What if currency doesn't exist?
    fn mc_db_client(&mut self, currency: Currency) -> &mut Self::McDbClient;

    fn get_tc_status(&mut self) -> AsyncOpResult<TcStatus>;
    fn set_direction_incoming(&mut self, move_token_hashed: MoveTokenHashed) -> AsyncOpResult<()>;
    fn set_direction_outgoing(
        &mut self,
        move_token: MoveToken,
        move_token_counter: u128,
    ) -> AsyncOpResult<()>;
    fn set_direction_outgoing_empty_incoming(
        &mut self,
        move_token: MoveToken,
        move_token_counter: u128,
    ) -> AsyncOpResult<()>;
    fn set_inconsistent(
        &mut self,
        local_reset_token: Signature,
        local_reset_move_token_counter: u128,
    ) -> AsyncOpResult<()>;

    /// Set remote terms for reset. Can only be called if we are in inconsistent state.
    fn set_inconsistent_remote_terms(
        &mut self,
        remote_reset_token: Signature,
        remote_reset_move_token_counter: u128,
    ) -> AsyncOpResult<()>;

    /// Add a new remote reset balance to the remote reset terms list
    /// Can only be called if we already called `set_inconsistent_remote_terms()`.
    fn add_remote_reset_balance(
        &mut self,
        currency: Currency,
        reset_balance: ResetBalance,
    ) -> AsyncOpResult<()>;

    /// Simulate outgoing token, to be used before an incoming reset move token (a remote reset)
    fn set_outgoing_from_inconsistent(&mut self, move_token: MoveToken) -> AsyncOpResult<()>;

    /// Simulate incoming token, to be used before an outgoing reset move token (a local reset)
    fn set_incoming_from_inconsistent(
        &mut self,
        move_token_hashed: MoveTokenHashed,
    ) -> AsyncOpResult<()>;

    fn get_move_token_counter(&mut self) -> AsyncOpResult<u128>;
    // fn set_move_token_counter(&mut self, move_token_counter: u128) -> AsyncOpResult<()>;

    /// Get currency's configured remote max debt
    fn get_remote_max_debt(&mut self, currency: Currency) -> AsyncOpResult<u128>;

    /// Return a sorted async iterator of all balances
    fn list_balances(&mut self) -> AsyncOpStream<(Currency, McBalance)>;

    /// Return a sorted async iterator of all local reset proposal balances
    /// Only relevant for inconsistent channels
    fn list_local_reset_balances(&mut self) -> AsyncOpStream<(Currency, ResetBalance)>;

    /// Return a sorted async iterator of all remote reset proposal balances
    /// Only relevant for inconsistent channels
    fn list_remote_reset_balances(&mut self) -> AsyncOpStream<(Currency, ResetBalance)>;

    fn is_local_currency(&mut self, currency: Currency) -> AsyncOpResult<bool>;
    fn is_remote_currency(&mut self, currency: Currency) -> AsyncOpResult<bool>;

    fn add_local_currency(&mut self, currency: Currency) -> AsyncOpResult<bool>;
    fn remove_local_currency(&mut self, currency: Currency) -> AsyncOpResult<bool>;

    fn add_remote_currency(&mut self, currency: Currency) -> AsyncOpResult<bool>;
    fn remove_remote_currency(&mut self, currency: Currency) -> AsyncOpResult<bool>;

    fn add_mutual_credit(&mut self, currency: Currency) -> AsyncOpResult<()>;
}
