#![warn(unused)]

use byteorder::{BigEndian, WriteBytesExt};
use crypto::hash;
use crypto::identity::verify_signature;
use super::types::{ResponseSendFunds, FailureSendFunds, PendingFriendRequest};

const FUND_SUCCESS_PREFIX: &[u8] = b"FUND_SUCCESS";
const FUND_FAILURE_PREFIX: &[u8] = b"FUND_FAILURE";

/// Create the buffer we sign over at the Response funds.
/// Note that the signature is not just over the Response funds bytes. The signed buffer also
/// contains information from the Request funds.
pub fn create_response_signature_buffer(response_send_funds: &ResponseSendFunds,
                        pending_request: &PendingFriendRequest) -> Vec<u8> {

    let mut sbuffer = Vec::new();

    // TODO: Add a const for this:
    sbuffer.extend_from_slice(&hash::sha_512_256(FUND_SUCCESS_PREFIX));

    let mut inner_blob = Vec::new();
    inner_blob.extend_from_slice(&pending_request.request_id);
    inner_blob.extend_from_slice(&pending_request.route.hash());
    inner_blob.extend_from_slice(&response_send_funds.rand_nonce);

    sbuffer.extend_from_slice(&hash::sha_512_256(&inner_blob));
    sbuffer.write_u128::<BigEndian>(pending_request.dest_payment).unwrap();
    sbuffer.extend_from_slice(&pending_request.invoice_id);

    sbuffer
}

/// Create the buffer we sign over at the Failure funds.
/// Note that the signature is not just over the Response funds bytes. The signed buffer also
/// contains information from the Request funds.
pub fn create_failure_signature_buffer(failure_send_funds: &FailureSendFunds,
                        pending_request: &PendingFriendRequest) -> Vec<u8> {

    let mut sbuffer = Vec::new();

    sbuffer.extend_from_slice(&hash::sha_512_256(FUND_FAILURE_PREFIX));
    sbuffer.extend_from_slice(&pending_request.request_id);
    sbuffer.extend_from_slice(&pending_request.route.hash());

    sbuffer.write_u128::<BigEndian>(pending_request.dest_payment).unwrap();
    sbuffer.extend_from_slice(&pending_request.invoice_id);
    sbuffer.extend_from_slice(&failure_send_funds.reporting_public_key);
    sbuffer.extend_from_slice(&failure_send_funds.rand_nonce);

    sbuffer
}

/// Verify a failure signature
pub fn verify_failure_signature(index: usize,
                            reporting_index: usize,
                            failure_send_funds: &FailureSendFunds,
                            pending_request: &PendingFriendRequest) -> Option<()> {

    let failure_signature_buffer = create_failure_signature_buffer(
                                        &failure_send_funds,
                                        &pending_request);
    let reporting_public_key = &failure_send_funds.reporting_public_key;
    // Make sure that the reporting_public_key is on the route:
    // TODO: Should we check that it is after us? Is it checked somewhere else?
    let _ = pending_request.route.pk_to_index(&reporting_public_key)?;

    if !verify_signature(&failure_signature_buffer, 
                     reporting_public_key, 
                     &failure_send_funds.signature) {
        return None;
    }
    Some(())
}


// TODO: How to test this?
