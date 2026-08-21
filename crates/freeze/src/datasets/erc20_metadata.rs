use crate::*;
use alloy::{primitives::Bytes, sol_types::SolCall};
use polars::prelude::*;
use std::collections::HashMap;

/// columns for transactions
#[triodion_macros::to_df(Datatype::Erc20Metadata)]
#[derive(Default)]
pub struct Erc20Metadata {
    n_rows: u64,
    block_number: Vec<u32>,
    erc20: Vec<Vec<u8>>,
    name: Vec<Option<String>>,
    symbol: Vec<Option<String>>,
    decimals: Vec<Option<u32>>,
    chain_id: Vec<u64>,
}

impl Dataset for Erc20Metadata {
    fn default_sort() -> Option<Vec<&'static str>> {
        Some(vec!["symbol", "block_number"])
    }

    fn default_blocks() -> Option<String> {
        Some("latest".to_string())
    }

    fn required_parameters() -> Vec<Dim> {
        vec![Dim::Address]
    }

    fn arg_aliases() -> Option<std::collections::HashMap<Dim, Dim>> {
        Some([(Dim::Contract, Dim::Address)].into_iter().collect())
    }
}

impl CollectByBlock for Erc20Metadata {
    type Response = (u32, Vec<u8>, Option<String>, Option<String>, Option<u32>);

    async fn extract(request: Params, source: Arc<Source>, _: Arc<Query>) -> R<Self::Response> {
        let block_number = request.ethers_block_number()?;
        let address = request.ethers_address()?;

        // Each read folds a *contract-level* refusal (revert, or an address with
        // no code) into `None`, while a *node-level* failure — pruned state on a
        // non-archive endpoint, a rate limit, a timeout — propagates via `?`.
        // Without that split, pointing this dataset at a non-archive RPC yields
        // a file of nulls under a "chunks errored: 0" banner.

        // name
        let call_data = ERC20::nameCall::SELECTOR.to_vec();
        let name = contract_read(source.call2(address, call_data, block_number).await)?
            .and_then(|output| decode_string_or_bytes32(&output));

        // symbol
        let call_data = ERC20::symbolCall::SELECTOR.to_vec();
        let symbol = contract_read(source.call2(address, call_data, block_number).await)?
            .and_then(|output| decode_string_or_bytes32(&output));

        // decimals
        let call_data = ERC20::decimalsCall::SELECTOR.to_vec();
        let decimals = contract_read(source.call2(address, call_data, block_number).await)?
            .and_then(|output| bytes_to_u32(output).ok());

        Ok((request.block_number()? as u32, request.address()?, name, symbol, decimals))
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::Erc20Metadata)?;
        let (block, address, name, symbol, decimals) = response;
        columns.n_rows += 1;
        store!(schema, columns, block_number, block);
        store!(schema, columns, erc20, address);
        store!(schema, columns, name, name);
        store!(schema, columns, symbol, symbol);
        store!(schema, columns, decimals, decimals);
        Ok(())
    }

    async fn collect_by_block(
        partition: Partition,
        source: Arc<Source>,
        query: Arc<Query>,
        inner_request_size: Option<u64>,
    ) -> R<HashMap<Datatype, DataFrame>> {
        if query.multicall {
            multicall_collect_by_block::<Self>(partition, source, query, inner_request_size).await
        } else {
            default_collect_by_block::<Self>(partition, source, query, inner_request_size).await
        }
    }
}

impl CollectByTransaction for Erc20Metadata {
    type Response = ();
}

impl MulticallBatchable for Erc20Metadata {
    fn calls_for_row(params: &Params, require_success: bool) -> R<Vec<Multicall3::Call3>> {
        let target = params.ethers_address()?;
        let allow_failure = !require_success;
        Ok(vec![
            Multicall3::Call3 {
                target,
                allowFailure: allow_failure,
                callData: Bytes::from(ERC20::nameCall::SELECTOR.to_vec()),
            },
            Multicall3::Call3 {
                target,
                allowFailure: allow_failure,
                callData: Bytes::from(ERC20::symbolCall::SELECTOR.to_vec()),
            },
            Multicall3::Call3 {
                target,
                allowFailure: allow_failure,
                callData: Bytes::from(ERC20::decimalsCall::SELECTOR.to_vec()),
            },
        ])
    }

    fn decode_row(params: &Params, results: &[Multicall3::Result]) -> R<Self::Response> {
        // `calls_for_row` emits exactly three calls; a shorter slice means the
        // node returned a malformed aggregate3, and indexing would panic the
        // worker task rather than surface that as an error.
        let [name_result, symbol_result, decimals_result] = results else {
            return Err(err("multicall returned the wrong number of results for row"))
        };
        let name = if name_result.success {
            decode_string_or_bytes32(&name_result.returnData)
        } else {
            None
        };
        let symbol = if symbol_result.success {
            decode_string_or_bytes32(&symbol_result.returnData)
        } else {
            None
        };
        let decimals = if decimals_result.success && !decimals_result.returnData.is_empty() {
            bytes_to_u32(alloy::primitives::Bytes::copy_from_slice(&decimals_result.returnData))
                .ok()
        } else {
            None
        };
        Ok((params.block_number()? as u32, params.address()?, name, symbol, decimals))
    }
}
