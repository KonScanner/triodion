use crate::*;
use alloy::{
    primitives::{Address, Bytes},
    sol_types::SolCall,
};
use polars::prelude::*;
use std::collections::HashMap;

/// columns for transactions
#[triodion_macros::to_df(Datatype::Erc721Metadata)]
#[derive(Default)]
pub struct Erc721Metadata {
    n_rows: u64,
    block_number: Vec<u32>,
    erc721: Vec<Vec<u8>>,
    name: Vec<Option<String>>,
    symbol: Vec<Option<String>>,
    chain_id: Vec<u64>,
}

impl Dataset for Erc721Metadata {
    fn default_sort() -> Option<Vec<&'static str>> {
        Some(vec!["symbol", "block_number"])
    }

    fn required_parameters() -> Vec<Dim> {
        vec![Dim::Address]
    }

    fn arg_aliases() -> Option<std::collections::HashMap<Dim, Dim>> {
        Some([(Dim::Contract, Dim::Address)].into_iter().collect())
    }
}

impl CollectByBlock for Erc721Metadata {
    type Response = (u32, Vec<u8>, Option<String>, Option<String>);

    async fn extract(request: Params, source: Arc<Source>, _: Arc<Query>) -> R<Self::Response> {
        let block_number = request.ethers_block_number()?;
        let address = request.ethers_address()?;

        // A contract-level refusal becomes `None`; a node-level failure
        // propagates so the chunk is reported as errored instead of silently
        // written out as nulls. See `contract_read`.

        // name
        let call_data = ERC721::nameCall::SELECTOR.to_vec();
        let name = contract_read(source.call2(address, call_data, block_number).await)?
            .and_then(|output| decode_string_or_bytes32(&output));

        // symbol
        let call_data = ERC721::symbolCall::SELECTOR.to_vec();
        let symbol = contract_read(source.call2(address, call_data, block_number).await)?
            .and_then(|output| decode_string_or_bytes32(&output));

        Ok((request.block_number()? as u32, request.address()?, name, symbol))
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::Erc721Metadata)?;
        let (block, address, name, symbol) = response;
        columns.n_rows += 1;
        store!(schema, columns, block_number, block);
        store!(schema, columns, erc721, address);
        store!(schema, columns, name, name);
        store!(schema, columns, symbol, symbol);
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

impl CollectByTransaction for Erc721Metadata {
    type Response = ();
}

impl MulticallBatchable for Erc721Metadata {
    fn calls_for_row(params: &Params, require_success: bool) -> R<Vec<Multicall3::Call3>> {
        let target = Address::from_slice(&params.address()?);
        let allow_failure = !require_success;
        Ok(vec![
            Multicall3::Call3 {
                target,
                allowFailure: allow_failure,
                callData: Bytes::from(ERC721::nameCall::SELECTOR.to_vec()),
            },
            Multicall3::Call3 {
                target,
                allowFailure: allow_failure,
                callData: Bytes::from(ERC721::symbolCall::SELECTOR.to_vec()),
            },
        ])
    }

    fn decode_row(params: &Params, results: &[Multicall3::Result]) -> R<Self::Response> {
        // `calls_for_row` emits exactly two calls; indexing a shorter slice
        // would panic the worker task instead of surfacing an error.
        let [name_result, symbol_result] = results else {
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
        Ok((params.block_number()? as u32, params.address()?, name, symbol))
    }
}
