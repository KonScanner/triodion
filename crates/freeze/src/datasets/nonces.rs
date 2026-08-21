use crate::*;
use alloy::{
    eips::BlockNumberOrTag,
    primitives::{Address, U64},
};
use polars::prelude::*;
use std::collections::HashMap;

/// columns for balances
#[triodion_macros::to_df(Datatype::Nonces)]
#[derive(Default)]
pub struct Nonces {
    n_rows: usize,
    block_number: Vec<u32>,
    address: Vec<Vec<u8>>,
    nonce: Vec<u64>,
    chain_id: Vec<u64>,
}

impl Dataset for Nonces {
    fn default_sort() -> Option<Vec<&'static str>> {
        Some(vec!["block_number", "address"])
    }

    fn required_parameters() -> Vec<Dim> {
        vec![Dim::Address]
    }

    fn default_blocks() -> Option<String> {
        Some("latest".to_string())
    }
}

type BlockTxAddressOutput = (u32, Option<Vec<u8>>, Vec<u8>, u64);

impl CollectByBlock for Nonces {
    type Response = BlockTxAddressOutput;

    async fn extract(request: Params, source: Arc<Source>, _: Arc<Query>) -> R<Self::Response> {
        let address = request.address()?;
        let block_number = request.block_number()? as u32;
        let output = source
            .get_transaction_count(Address::from_slice(&address), block_number.into())
            .await?;
        Ok((block_number, None, address, output))
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::Nonces)?;
        process_nonce(columns, response, schema)
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

impl CollectByTransaction for Nonces {
    type Response = ();
}

/// Nonces batch at the transport and nowhere else.
///
/// This is the one account field the state-override trick cannot reach. The EVM
/// exposes an account's balance (`BALANCE`), its code (`EXTCODESIZE`,
/// `EXTCODEHASH`, `EXTCODECOPY`) and — from inside it — its storage (`SLOAD`),
/// but there is no opcode that reads another account's nonce. No injected
/// bytecode batches this, on any node, at any block. `eth_getProof` returns a
/// nonce, but one account per call, so it buys nothing here either.
///
/// So the only lever is JSON-RPC batching: same request count reduction, no
/// requirements on the node beyond JSON-RPC itself.
impl RpcBatchable for Nonces {
    type Param = (Address, BlockNumberOrTag);
    type Item = U64;

    fn method() -> &'static str {
        "eth_getTransactionCount"
    }

    fn param_for_row(params: &Params) -> R<Self::Param> {
        // `ethers_address`, never `Address::from_slice`, which PANICS on a width
        // mismatch — and `--address` is hex-decoded with no length check. This
        // runs once per row of a whole chunk, so a panic would abort a hundred
        // rows where the per-row path aborted one. As an error the chunk demotes
        // and the per-row path reports that address as it always did.
        Ok((params.ethers_address()?, BlockNumberOrTag::Number(params.block_number()?)))
    }

    fn decode_row(params: &Params, item: Self::Item) -> R<Self::Response> {
        Ok((params.block_number()? as u32, None, params.address()?, item.to::<u64>()))
    }
}

fn process_nonce(columns: &mut Nonces, data: BlockTxAddressOutput, schema: &Table) -> R<()> {
    let (block, _tx, address, output) = data;
    columns.n_rows += 1;
    store!(schema, columns, block_number, block);
    store!(schema, columns, address, address);
    store!(schema, columns, nonce, output);
    Ok(())
}
