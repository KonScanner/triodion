use crate::*;
use alloy::{
    primitives::{Bytes, U256},
    sol_types::SolCall,
};
use polars::prelude::*;
use std::collections::HashMap;

/// columns for transactions
#[triodion_macros::to_df(Datatype::Erc20Balances)]
#[derive(Default)]
pub struct Erc20Balances {
    n_rows: u64,
    block_number: Vec<u32>,
    erc20: Vec<Vec<u8>>,
    address: Vec<Vec<u8>>,
    balance: Vec<Option<U256>>,
    chain_id: Vec<u64>,
}

impl Dataset for Erc20Balances {
    fn default_blocks() -> Option<String> {
        Some("latest".to_string())
    }

    fn required_parameters() -> Vec<Dim> {
        vec![Dim::Contract, Dim::Address]
    }
}

impl CollectByBlock for Erc20Balances {
    type Response = (u32, Vec<u8>, Vec<u8>, Option<U256>);

    async fn extract(request: Params, source: Arc<Source>, _: Arc<Query>) -> R<Self::Response> {
        let signature = ERC20::balanceOfCall::SELECTOR;
        let mut call_data = signature.clone().to_vec();
        call_data.extend(vec![0; 12]);
        call_data.extend(request.address()?);
        let block_number = request.ethers_block_number()?;
        let contract = request.ethers_contract()?;
        // A revert, or an address with no code, means "no balance to report"
        // and becomes a null. A node that could not serve the state propagates
        // so the chunk is counted as errored rather than written out as nulls.
        let output = contract_read(source.call2(contract, call_data, block_number).await)?;
        let balance = output.and_then(|bytes| decode_u256_word(&bytes));
        Ok((request.block_number()? as u32, request.contract()?, request.address()?, balance))
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::Erc20Balances)?;
        let (block, erc20, address, balance) = response;
        columns.n_rows += 1;
        store!(schema, columns, block_number, block);
        store!(schema, columns, erc20, erc20);
        store!(schema, columns, address, address);
        store!(schema, columns, balance, balance);
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

impl CollectByTransaction for Erc20Balances {
    type Response = ();
}

impl MulticallBatchable for Erc20Balances {
    fn calls_for_row(params: &Params, require_success: bool) -> R<Vec<Multicall3::Call3>> {
        let owner = params.ethers_address()?;
        let contract = params.ethers_contract()?;
        let call_data = ERC20::balanceOfCall { owner }.abi_encode();
        Ok(vec![Multicall3::Call3 {
            target: contract,
            allowFailure: !require_success,
            callData: Bytes::from(call_data),
        }])
    }

    fn decode_row(params: &Params, results: &[Multicall3::Result]) -> R<Self::Response> {
        // Indexing would panic the worker task on a short aggregate3 return.
        let r = results.first().ok_or_else(|| err("multicall returned no result for row"))?;
        let balance = if r.success { decode_u256_word(&r.returnData) } else { None };
        Ok((params.block_number()? as u32, params.contract()?, params.address()?, balance))
    }
}
