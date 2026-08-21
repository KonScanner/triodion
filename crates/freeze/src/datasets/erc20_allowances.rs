use crate::*;
use alloy::{
    primitives::{Bytes, U256},
    sol_types::SolCall,
};
use polars::prelude::*;
use std::collections::HashMap;

/// columns for erc20_allowances
///
/// One row per `(token, owner, spender)` per block: how much of the owner's
/// balance the spender is currently permitted to move.
///
/// # This is the state, not the history
///
/// Two datasets answer questions about ERC-20 approvals, and they answer
/// different ones. Pick by the question, not by the name:
///
/// * [`Erc20Approvals`] reads the `Approval` **event** log. It gives every approval *change* in a
///   block range, with the transaction that made it, and needs no owner or spender named in advance
///   — it discovers them. Use it to ask "who approved what, and when".
/// * This dataset issues an `allowance(owner, spender)` **call**. It gives the value in force at a
///   block, whatever sequence of approvals produced it, and requires the owner and spender to be
///   named. Use it to ask "what can be spent right now".
///
/// The event log alone cannot answer the second question. An allowance also
/// falls when the spender spends — `transferFrom` decrements it without
/// emitting `Approval` on most implementations — so replaying approval events
/// overstates what is actually spendable. Only the call reads the truth.
///
/// # On "unlimited" approvals
///
/// The common way to grant an unbounded allowance is to set it to
/// `type(uint256).max`. It is a convention, not a rule: some tokens and
/// front-ends use `2^255 - 1`, and some decrement from whatever was set. The
/// `allowance` column is therefore the raw value, with no derived "unlimited"
/// flag — testing `== U256::MAX` would miss the other conventions, and
/// thresholding at some round number would be this tool inventing a fact.
/// Threshold it yourself, in the direction your analysis needs.
#[triodion_macros::to_df(Datatype::Erc20Allowances)]
#[derive(Default)]
pub struct Erc20Allowances {
    n_rows: u64,
    block_number: Vec<u32>,
    // The token, `Dim::Contract`.
    erc20: Vec<Vec<u8>>,
    // The owner whose tokens may be spent, `Dim::FromAddress`. Named
    // `from_address` rather than `owner` so a row here joins directly against
    // [`Erc20Approvals`], which stores the `Approval` event's indexed owner in
    // a column of the same name. The whole point of having both datasets is
    // being able to put them side by side.
    from_address: Vec<Vec<u8>>,
    // The spender permitted to move them, `Dim::ToAddress`. Matches
    // `Erc20Approvals::to_address` for the same reason.
    to_address: Vec<Vec<u8>>,
    // 0 and null are different facts. A successful read of 0 means the spender
    // may move nothing — a real measurement, and the state after a revocation.
    // A null means the call did not return an allowance at all: the address has
    // no code at this block, or is not an ERC-20. Filling the null with 0 would
    // report "approved for nothing" where the right answer is "no such token".
    allowance: Vec<Option<U256>>,
    chain_id: Vec<u64>,
}

impl Dataset for Erc20Allowances {
    fn default_blocks() -> Option<String> {
        Some("latest".to_string())
    }

    fn aliases() -> Vec<&'static str> {
        vec!["allowances"]
    }

    fn required_parameters() -> Vec<Dim> {
        // All three are required, and none of them can be inferred. An
        // `allowance()` call takes an owner and a spender; there is no state
        // read that enumerates the spenders a token has approved, because the
        // mapping's keys are not recoverable from its slots. A user who does
        // not know the pairs yet wants `erc20_approvals`, which finds them in
        // the event log, and can then feed them back here.
        vec![Dim::Contract, Dim::FromAddress, Dim::ToAddress]
    }

    fn default_sort() -> Option<Vec<&'static str>> {
        Some(vec!["block_number", "erc20", "from_address", "to_address"])
    }
}

type BlockErc20OwnerSpenderAllowance = (u32, Vec<u8>, Vec<u8>, Vec<u8>, Option<U256>);

impl CollectByBlock for Erc20Allowances {
    type Response = BlockErc20OwnerSpenderAllowance;

    async fn extract(request: Params, source: Arc<Source>, _: Arc<Query>) -> R<Self::Response> {
        let owner = request.ethers_from_address()?;
        let spender = request.ethers_to_address()?;
        let contract = request.ethers_contract()?;
        let block_number = request.ethers_block_number()?;
        let call_data = ERC20::allowanceCall { owner, spender }.abi_encode();
        // A revert, or an address with no code, means "no allowance to report"
        // and becomes a null. A node that could not serve the state propagates,
        // so the chunk is counted as errored rather than written out as nulls.
        let output = contract_read(source.call2(contract, call_data, block_number).await)?;
        let allowance = output.and_then(|bytes| decode_u256_word(&bytes));
        Ok((
            request.block_number()? as u32,
            request.contract()?,
            request.from_address()?,
            request.to_address()?,
            allowance,
        ))
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::Erc20Allowances)?;
        process_allowance(response, columns, schema)
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

impl CollectByTransaction for Erc20Allowances {
    type Response = ();
}

/// Allowances aggregate through Multicall3 like any other `eth_call` dataset.
///
/// One `(token, owner, spender)` triple is one `allowance()` call, and calls
/// sharing a block aggregate into a single `aggregate3`. There is no state
/// override to be had here: the extractor trick reads *slots*, and finding the
/// slot of `allowances[owner][spender]` needs the mapping's base slot, which
/// varies per token and is not discoverable from the ABI.
impl MulticallBatchable for Erc20Allowances {
    fn calls_for_row(params: &Params, require_success: bool) -> R<Vec<Multicall3::Call3>> {
        let owner = params.ethers_from_address()?;
        let spender = params.ethers_to_address()?;
        let contract = params.ethers_contract()?;
        let call_data = ERC20::allowanceCall { owner, spender }.abi_encode();
        Ok(vec![Multicall3::Call3 {
            target: contract,
            allowFailure: !require_success,
            callData: Bytes::from(call_data),
        }])
    }

    fn decode_row(params: &Params, results: &[Multicall3::Result]) -> R<Self::Response> {
        // Indexing would panic the worker task on a short aggregate3 return.
        let r = results.first().ok_or_else(|| err("multicall returned no result for row"))?;
        let allowance = if r.success { decode_u256_word(&r.returnData) } else { None };
        Ok((
            params.block_number()? as u32,
            params.contract()?,
            params.from_address()?,
            params.to_address()?,
            allowance,
        ))
    }
}

fn process_allowance(
    response: BlockErc20OwnerSpenderAllowance,
    columns: &mut Erc20Allowances,
    schema: &Table,
) -> R<()> {
    let (block, erc20, from_address, to_address, allowance) = response;
    columns.n_rows += 1;
    store!(schema, columns, block_number, block);
    store!(schema, columns, erc20, erc20);
    store!(schema, columns, from_address, from_address);
    store!(schema, columns, to_address, to_address);
    store!(schema, columns, allowance, allowance);
    Ok(())
}
