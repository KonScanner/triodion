use crate::*;
use alloy::{eips::BlockNumberOrTag, primitives::U256, rpc::types::BlockTransactionsKind};
use polars::prelude::*;
use std::collections::HashMap;

/// columns for transactions
#[triodion_macros::to_df(Datatype::Blocks)]
#[derive(Default)]
pub struct Blocks {
    n_rows: u64,
    block_hash: Vec<Option<Vec<u8>>>,
    parent_hash: Vec<Vec<u8>>,
    uncles_hash: Vec<Vec<u8>>,
    author: Vec<Option<Vec<u8>>>,
    state_root: Vec<Vec<u8>>,
    transactions_root: Vec<Vec<u8>>,
    receipts_root: Vec<Vec<u8>>,
    block_number: Vec<Option<u32>>,
    gas_used: Vec<u64>,
    gas_limit: Vec<u64>,
    extra_data: Vec<Vec<u8>>,
    logs_bloom: Vec<Option<Vec<u8>>>,
    timestamp: Vec<u32>,
    difficulty: Vec<u64>,
    total_difficulty: Vec<Option<U256>>,
    size: Vec<Option<u64>>,
    mix_hash: Vec<Option<Vec<u8>>>,
    nonce: Vec<Option<Vec<u8>>>,
    base_fee_per_gas: Vec<Option<u64>>,
    withdrawals_root: Vec<Option<Vec<u8>>>,
    // Per-block withdrawal aggregates (EIP-4895, post-Shanghai). For pre-Shanghai
    // blocks `block.withdrawals` is `None` and we store 0/0 — the protocol simply
    // hadn't enabled withdrawals yet; pair with `timestamp` to disambiguate
    // "0 because no withdrawals this block" from "0 because pre-fork".
    withdrawals_count: Vec<u32>,
    withdrawals_amount_gwei: Vec<u64>,
    // EIP-4844 (Cancun). `None` before the fork, and on chains that never
    // enabled blobs — never 0, which would claim the block posted no blob gas
    // when in fact the concept did not exist.
    blob_gas_used: Vec<Option<u64>>,
    excess_blob_gas: Vec<Option<u64>>,
    // EIP-4788 (Cancun): the beacon block root of the parent slot, which is
    // what lets an execution-layer query join to consensus-layer state.
    parent_beacon_block_root: Vec<Option<Vec<u8>>>,
    // EIP-7685 (Prague): commitment over the block's execution requests
    // (EIP-6110 deposits, EIP-7002 withdrawals, EIP-7251 consolidations).
    requests_hash: Vec<Option<Vec<u8>>>,
    // Arbitrum-only header fields. See `types::chains::arbitrum`.
    // `l1_block_number` is the L1 block this L2 block was sequenced against;
    // `send_root` / `send_count` track the L2->L1 outbox accumulator.
    l1_block_number: Vec<Option<u64>>,
    send_root: Vec<Option<Vec<u8>>>,
    send_count: Vec<Option<u64>>,
    chain_id: Vec<u64>,
}

impl Dataset for Blocks {
    fn default_columns() -> Option<Vec<&'static str>> {
        Some(vec![
            "block_number",
            "block_hash",
            "timestamp",
            "author",
            "gas_used",
            "extra_data",
            "base_fee_per_gas",
            "withdrawals_count",
            "withdrawals_amount_gwei",
            "chain_id",
        ])
    }
}

impl CollectByBlock for Blocks {
    type Response = RpcBlock;

    async fn extract(request: Params, source: Arc<Source>, _: Arc<Query>) -> R<Self::Response> {
        let block = source
            .get_block(request.block_number()?, BlockTransactionsKind::Hashes)
            .await?
            .ok_or(CollectError::CollectError("block not found".to_string()))?;
        Ok(block)
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::Blocks)?;
        process_block(response, columns, schema)
    }

    async fn collect_by_block(
        partition: Partition,
        source: Arc<Source>,
        query: Arc<Query>,
        inner_request_size: Option<u64>,
    ) -> R<HashMap<Datatype, DataFrame>> {
        if query.batch_rpc_calls {
            rpc_batch_collect_by_block::<Self>(partition, source, query, inner_request_size).await
        } else {
            default_collect_by_block::<Self>(partition, source, query, inner_request_size).await
        }
    }
}

impl CollectByTransaction for Blocks {
    type Response = RpcBlock;

    async fn extract(request: Params, source: Arc<Source>, _: Arc<Query>) -> R<Self::Response> {
        let transaction = source
            .get_transaction_by_hash(request.ethers_transaction_hash()?)
            .await?
            .ok_or(CollectError::CollectError("transaction not found".to_string()))?;
        let block = source
            .get_block_by_hash(
                transaction.block_hash.ok_or(err("no block block_hash found"))?,
                BlockTransactionsKind::Hashes,
            )
            .await?
            .ok_or(CollectError::CollectError("block not found".to_string()))?;
        Ok(block)
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::Blocks)?;
        process_block(response, columns, schema)
    }
}

/// A hundred headers per request instead of a hundred requests.
///
/// Every row of this dataset is one block, and the per-row path spends one
/// `eth_getBlockByNumber` on each — so a year of mainnet is 2.6 million HTTP
/// requests to fetch 2.6 million headers that would fit in 26,000 of them. No
/// state override is involved and no aggregator contract exists for headers;
/// the whole win is in the envelope.
///
/// `fullTransactions` stays `false`, matching [`CollectByBlock::extract`]:
/// this dataset stores the transaction *root*, never the bodies, so asking for
/// them would multiply the response size for columns nothing reads.
///
/// A block the node does not have comes back as JSON `null`, which is why
/// [`Self::Item`] is an `Option`. That is a real answer to a real question, not
/// a transport failure, so it becomes the same "block not found" error the
/// per-row path raises rather than demoting the whole chunk to retry it.
impl RpcBatchable for Blocks {
    type Param = (BlockNumberOrTag, bool);
    type Item = Option<RpcBlock>;

    fn method() -> &'static str {
        "eth_getBlockByNumber"
    }

    fn param_for_row(params: &Params) -> R<Self::Param> {
        Ok((BlockNumberOrTag::Number(params.block_number()?), false))
    }

    fn decode_row(params: &Params, item: Self::Item) -> R<Self::Response> {
        item.ok_or_else(|| {
            let n = params.block_number.map_or("?".to_string(), |n| n.to_string());
            CollectError::CollectError(format!("block not found: {n}"))
        })
    }
}

/// process block into columns
///
/// Takes an [`RpcBlock`] (alloy's `AnyRpcBlock`) rather than an
/// `Ethereum`-typed `Block`, so the same function serves mainnet, the OP stack
/// and the Arbitrum stack. Fields the chain does not define are read as `None`
/// out of the header's optional fields or the block's extra-fields map.
pub(crate) fn process_block(block: RpcBlock, columns: &mut Blocks, schema: &Table) -> R<()> {
    columns.n_rows += 1;

    // Chain-specific header fields ride in the block's extra-fields map:
    // `AnyHeader` is `#[serde(flatten)]`ed into the block object, so anything
    // it does not name lands here rather than being dropped.
    let extra = block.other.clone();
    let block = block.into_inner();

    store!(schema, columns, block_hash, Some(block.header.hash.to_vec()));
    store!(schema, columns, parent_hash, block.header.parent_hash.0.to_vec());
    store!(
        schema,
        columns,
        uncles_hash,
        block.uncles.into_iter().flat_map(|s| s.to_vec()).collect()
    );
    store!(schema, columns, author, Some(block.header.beneficiary.to_vec()));
    store!(schema, columns, state_root, block.header.state_root.0.to_vec());
    store!(schema, columns, transactions_root, block.header.transactions_root.0.to_vec());
    store!(schema, columns, receipts_root, block.header.receipts_root.0.to_vec());
    store!(schema, columns, block_number, Some(block.header.number as u32));
    store!(schema, columns, gas_used, block.header.gas_used);
    store!(schema, columns, gas_limit, block.header.gas_limit);
    store!(schema, columns, extra_data, block.header.extra_data.to_vec());
    store!(schema, columns, logs_bloom, Some(block.header.logs_bloom.to_vec()));
    store!(schema, columns, timestamp, block.header.timestamp as u32);
    store!(schema, columns, difficulty, block.header.difficulty.wrapping_to::<u64>());
    store!(schema, columns, total_difficulty, block.header.total_difficulty);
    store!(schema, columns, base_fee_per_gas, block.header.base_fee_per_gas);
    store!(schema, columns, size, block.header.size.map(|v| v.wrapping_to::<u64>()));
    // `mix_hash` and `nonce` are `Option` on a cross-chain header: Arbitrum
    // reuses both for its own bookkeeping and some chains omit them outright.
    store!(schema, columns, mix_hash, block.header.mix_hash.map(|x| x.to_vec()));
    store!(schema, columns, nonce, block.header.nonce.map(|x| x.0.to_vec()));
    store!(schema, columns, withdrawals_root, block.header.withdrawals_root.map(|x| x.0.to_vec()));
    let (w_count, w_amount) = match block.withdrawals.as_ref() {
        Some(ws) => (ws.len() as u32, ws.iter().map(|w| w.amount).sum::<u64>()),
        None => (0u32, 0u64),
    };
    store!(schema, columns, withdrawals_count, w_count);
    store!(schema, columns, withdrawals_amount_gwei, w_amount);

    store!(schema, columns, blob_gas_used, block.header.blob_gas_used);
    store!(schema, columns, excess_blob_gas, block.header.excess_blob_gas);
    store!(
        schema,
        columns,
        parent_beacon_block_root,
        block.header.parent_beacon_block_root.map(|x| x.0.to_vec())
    );
    store!(schema, columns, requests_hash, block.header.requests_hash.map(|x| x.0.to_vec()));

    store!(schema, columns, l1_block_number, other_u64(&extra, arbitrum::L1_BLOCK_NUMBER));
    store!(schema, columns, send_root, other_bytes(&extra, arbitrum::SEND_ROOT));
    store!(schema, columns, send_count, other_u64(&extra, arbitrum::SEND_COUNT));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::types::wire_fixtures::{arbitrum_one_block, op_mainnet_block};

    fn collect(json: serde_json::Value) -> Blocks {
        let block: RpcBlock = serde_json::from_value(json).expect("block fixture deserializes");
        let schema = Datatype::Blocks
            .table_schema(
                &[U256Type::String],
                &ColumnEncoding::Hex,
                &None,
                &None,
                &Some(vec!["all".to_string()]),
                None,
                None,
            )
            .expect("every column is nameable");
        let mut columns = Blocks::default();
        process_block(block, &mut columns, &schema).expect("header maps to columns");
        columns
    }

    #[test]
    fn an_op_stack_header_carries_the_cancun_fields() {
        let columns = collect(op_mainnet_block());
        assert_eq!(columns.n_rows, 1);
        assert_eq!(columns.blob_gas_used[0], Some(0));
        assert_eq!(columns.excess_blob_gas[0], Some(0));
        assert!(columns.parent_beacon_block_root[0].is_some());
        // Arbitrum-only fields stay null on an OP-stack chain.
        assert_eq!(columns.send_root[0], None);
        assert_eq!(columns.l1_block_number[0], None);
    }

    #[test]
    fn an_arbitrum_header_carries_its_own_fields_and_omits_the_cancun_ones() {
        let columns = collect(arbitrum_one_block());
        assert_eq!(columns.l1_block_number[0], Some(19_662_017));
        assert_eq!(columns.send_count[0], Some(115_724));
        assert_eq!(
            columns.send_root[0].as_ref().map(alloy::hex::encode),
            Some("f0f401b0308982116f63f8af9eac3d2ddf7545cfab79a3e132538c36c1036557".to_string())
        );
        // Arbitrum never enabled blobs or EIP-4788. `None`, not `0`: the block
        // did not spend zero blob gas, the concept does not exist there.
        assert_eq!(columns.blob_gas_used[0], None);
        assert_eq!(columns.parent_beacon_block_root[0], None);
        assert_eq!(columns.requests_hash[0], None);
    }

    #[test]
    fn an_arbitrum_header_keeps_its_optional_nonce_and_mix_hash() {
        // These are `Option` on a cross-chain header. Arbitrum populates both,
        // with its own meanings, so a `None` here would be a silent loss.
        let columns = collect(arbitrum_one_block());
        assert!(columns.mix_hash[0].is_some());
        assert!(columns.nonce[0].is_some());
    }
}
