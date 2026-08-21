use crate::*;
use alloy::{primitives::U256, rpc::types::BlockTransactionsKind};
use polars::prelude::*;

/// columns for withdrawals
///
/// One row per EIP-4895 validator withdrawal.
///
/// [`Blocks`] carries only `withdrawals_count` and `withdrawals_amount_gwei`.
/// An aggregate cannot be taken apart again: which validator was paid, and how
/// much each was paid, is gone by the time that row is written. This dataset
/// keeps the individual records.
///
/// A withdrawal is credited by the protocol, not sent by anyone. It has no
/// sender, no gas cost, no receipt and no transaction hash, and it appears in
/// no other dataset — `traces` and `native_transfers` both read execution, and
/// a withdrawal never executes. The block body is the only place it exists.
#[triodion_macros::to_df(Datatype::Withdrawals)]
#[derive(Default)]
pub struct Withdrawals {
    n_rows: u64,
    block_number: Vec<u32>,
    block_hash: Vec<Option<Vec<u8>>>,
    timestamp: Vec<u32>,
    // Issued by the consensus layer and monotonic across the whole chain, not
    // restarted per block. It is the stable key for a withdrawal.
    withdrawal_index: Vec<u64>,
    validator_index: Vec<u64>,
    address: Vec<Vec<u8>>,
    // Gwei is the protocol's own unit here, and the name says so. Reading this
    // as wei understates every balance by a factor of a billion.
    amount_gwei: Vec<u64>,
    // The same amount in wei, for joining against value columns that are in
    // wei. `U256` and not `u64`: 2048 ETH in wei does not fit in 64 bits.
    amount_wei: Vec<U256>,
    chain_id: Vec<u64>,
}

impl Dataset for Withdrawals {
    fn default_columns() -> Option<Vec<&'static str>> {
        Some(vec![
            "block_number",
            "timestamp",
            "withdrawal_index",
            "validator_index",
            "address",
            "amount_gwei",
            "chain_id",
        ])
    }

    fn default_sort() -> Option<Vec<&'static str>> {
        Some(vec!["block_number", "withdrawal_index"])
    }
}

impl CollectByBlock for Withdrawals {
    type Response = RpcBlock;

    async fn extract(request: Params, source: Arc<Source>, _: Arc<Query>) -> R<Self::Response> {
        // Transaction hashes only: withdrawals live beside the transaction
        // list, never inside it, so full bodies would be paid for and unread.
        source
            .get_block(request.block_number()?, BlockTransactionsKind::Hashes)
            .await?
            .ok_or_else(|| err("block not found"))
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::Withdrawals)?;
        process_withdrawals(response, columns, schema)
    }
}

// A withdrawal has no transaction to be collected by. Unlike `blocks`, which
// can answer "the block containing this transaction", there is no relationship
// here to resolve: `-t` would silently mean "withdrawals that happen to share a
// block with this transaction", which is not a question anyone asked.
impl CollectByTransaction for Withdrawals {
    type Response = ();
}

/// Explode a block's withdrawal list into one row each.
fn process_withdrawals(block: RpcBlock, columns: &mut Withdrawals, schema: &Table) -> R<()> {
    // `None` before Shanghai, and on chains that never enabled withdrawals.
    // Either way there is nothing to explode, and no row is the right answer —
    // as distinct from `blocks`, where the aggregate columns must still hold a
    // value and record 0.
    let Some(withdrawals) = block.withdrawals.as_ref() else { return Ok(()) };
    let block_number = block.header.number as u32;
    let block_hash = block.header.hash.to_vec();
    let timestamp = block.header.timestamp as u32;

    for withdrawal in withdrawals.iter() {
        columns.n_rows += 1;
        store!(schema, columns, block_number, block_number);
        store!(schema, columns, block_hash, Some(block_hash.clone()));
        store!(schema, columns, timestamp, timestamp);
        store!(schema, columns, withdrawal_index, withdrawal.index);
        store!(schema, columns, validator_index, withdrawal.validator_index);
        store!(schema, columns, address, withdrawal.address.to_vec());
        store!(schema, columns, amount_gwei, withdrawal.amount);
        // Widen before multiplying. `amount * 1_000_000_000` overflows `u64`
        // above ~18.4 ETH, which is well inside the range a validator can hold.
        store!(
            schema,
            columns,
            amount_wei,
            U256::from(withdrawal.amount) * U256::from(1_000_000_000u64)
        );
    }
    Ok(())
}
