use crate::*;
use polars::prelude::*;

/// columns for withdrawal_requests
///
/// One row per EIP-7002 withdrawal request, from Prague onward.
///
/// A withdrawal *request* is not a withdrawal. It is a validator exit or
/// partial withdrawal triggered from the execution layer, by a call to the
/// withdrawal predeploy. The payment it eventually causes appears later, in a
/// different block, as a row in [`Withdrawals`]. The two are related by
/// validator, never by block.
///
/// EIP-7002 exists so that the holder of the withdrawal credentials can exit a
/// validator without the validator key. That makes the requester an execution
/// address, which is what `source_address` records.
///
/// Like the other request datasets this reads the consensus block, because the
/// execution layer commits to requests in `requests_hash` and does not serve
/// them.
#[triodion_macros::to_df(Datatype::WithdrawalRequests)]
#[derive(Default)]
pub struct WithdrawalRequests {
    n_rows: u64,
    block_number: Vec<u32>,
    timestamp: Vec<u32>,
    slot: Vec<u64>,
    epoch: Vec<Option<u64>>,
    proposer_index: Vec<u64>,
    // Position within this block's withdrawal-request list.
    request_index: Vec<u32>,
    // The execution address that submitted the request, 20 bytes. It must hold
    // the validator's withdrawal credentials for the request to be honoured.
    source_address: Vec<Vec<u8>>,
    // BLS public key of the validator to withdraw from, 48 bytes.
    validator_pubkey: Vec<Vec<u8>>,
    // Gwei. Zero does not mean "nothing": EIP-7002 encodes a full exit as an
    // amount of zero, and a partial withdrawal as the amount to leave. Summing
    // this column across full exits therefore reports zero for the largest
    // withdrawals on the chain, which is why `is_full_exit` exists beside it.
    amount_gwei: Vec<u64>,
    // Derived, not reported: `amount_gwei == 0`. Present so the zero above
    // cannot be read as an empty request.
    is_full_exit: Vec<bool>,
    chain_id: Vec<u64>,
}

impl Dataset for WithdrawalRequests {
    fn default_columns() -> Option<Vec<&'static str>> {
        Some(vec![
            "block_number",
            "timestamp",
            "slot",
            "request_index",
            "source_address",
            "validator_pubkey",
            "amount_gwei",
            "is_full_exit",
            "chain_id",
        ])
    }

    fn default_sort() -> Option<Vec<&'static str>> {
        Some(vec!["block_number", "request_index"])
    }
}

impl CollectByBlock for WithdrawalRequests {
    type Response = beacon_requests::BlockRequests;

    async fn extract(request: Params, source: Arc<Source>, _: Arc<Query>) -> R<Self::Response> {
        beacon_requests::fetch(request, source).await
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::WithdrawalRequests)?;
        let Some(requests) = response.requests.as_ref() else { return Ok(()) };
        for (index, withdrawal) in requests.withdrawals.iter().enumerate() {
            columns.n_rows += 1;
            store!(schema, columns, block_number, response.block_number);
            store!(schema, columns, timestamp, response.timestamp);
            store!(schema, columns, slot, response.slot);
            store!(schema, columns, epoch, response.epoch);
            store!(schema, columns, proposer_index, response.proposer_index);
            store!(schema, columns, request_index, index as u32);
            store!(schema, columns, source_address, withdrawal.source_address.clone());
            store!(schema, columns, validator_pubkey, withdrawal.validator_pubkey.clone());
            store!(schema, columns, amount_gwei, withdrawal.amount);
            store!(schema, columns, is_full_exit, withdrawal.amount == 0);
        }
        Ok(())
    }
}

impl CollectByTransaction for WithdrawalRequests {
    type Response = ();
}
