use crate::*;
use alloy::{
    primitives::{Bytes, U256},
    sol_types::SolCall,
};
use polars::prelude::*;
use std::collections::HashMap;

/// columns for erc2612_nonces
///
/// One row per (token, owner) per block: the ERC-2612 permit nonce.
///
/// ERC-2612 lets a token owner approve a spender with an off-chain signature.
/// The `permit` call that redeems that signature emits the ordinary ERC-20
/// `Approval` event and nothing else — the standard defines no event of its
/// own. In [`Erc20Approvals`] a permit-granted approval is therefore
/// indistinguishable from one granted by an on-chain `approve()`: same topic0,
/// same owner, same spender, same value.
///
/// The nonce is the only on-chain counter of how many permits an owner has
/// signed for a token. Read it at two blocks and the difference is the number
/// of permits redeemed in between.
///
/// This dataset gives that count. It does **not** give the linkage: it cannot
/// attribute a specific `Approval` row to a permit. Answering that needs the
/// transaction's calldata — the `permit` selector, possibly nested inside a
/// router's own multicall — which is a different question from the one asked
/// here.
#[triodion_macros::to_df(Datatype::Erc2612Nonces)]
#[derive(Default)]
pub struct Erc2612Nonces {
    n_rows: u64,
    block_number: Vec<u32>,
    // The token, `Dim::Contract`.
    erc20: Vec<Vec<u8>>,
    // The permit owner, `Dim::Address`. The nonce is per owner per token, so
    // both dimensions are required to name a row.
    address: Vec<Vec<u8>>,
    // 0 and null are different facts here. A successful read of 0 means the
    // owner has signed no permit for this token yet — a real measurement. A
    // null means the call did not return a nonce at all, so the token has no
    // ERC-2612 support and the concept does not exist for it. Filling the null
    // with 0 would report "never permitted" for tokens that can never permit.
    nonce: Vec<Option<U256>>,
    // EIP-712 domain separator, opt-in because it is constant per token per
    // chain and repeats identically on every row. Its null is the cheapest
    // signal that a token is not a permit token: `DOMAIN_SEPARATOR()` reverts
    // on anything that does not implement ERC-2612.
    domain_separator: Vec<Option<Vec<u8>>>,
    chain_id: Vec<u64>,
}

impl Dataset for Erc2612Nonces {
    fn default_columns() -> Option<Vec<&'static str>> {
        Some(vec![
            "block_number",
            "erc20",
            "address",
            "nonce",
            // "domain_separator",
            "chain_id",
        ])
    }

    fn default_sort() -> Option<Vec<&'static str>> {
        Some(vec!["block_number", "erc20", "address"])
    }

    fn required_parameters() -> Vec<Dim> {
        vec![Dim::Contract, Dim::Address]
    }

    fn default_blocks() -> Option<String> {
        Some("latest".to_string())
    }
}

type BlockTokenOwnerNonce = (u32, Vec<u8>, Vec<u8>, Option<U256>, Option<Vec<u8>>);

impl CollectByBlock for Erc2612Nonces {
    type Response = BlockTokenOwnerNonce;

    async fn extract(request: Params, source: Arc<Source>, query: Arc<Query>) -> R<Self::Response> {
        let block_number = request.ethers_block_number()?;
        let token = request.ethers_contract()?;
        let owner_bytes = request.address()?;
        let owner = request.ethers_address()?;

        // `contract_read` keeps the two failure modes apart. A revert, or an
        // address with no code, is a real answer about the chain and becomes a
        // null cell; a node that could not serve the state — pruned history on
        // a non-archive endpoint, a rate limit, a timeout — propagates, so the
        // chunk is counted as errored instead of written out as nulls.

        let call_data = ERC2612::noncesCall { owner }.abi_encode();
        let output = contract_read(source.call2(token, call_data, block_number).await)?;
        let nonce = output.and_then(|bytes| decode_u256_word(&bytes));

        // Skipped unless the column was actually selected. It is off by default
        // and constant per (token, chain), so issuing it regardless doubled the
        // request count of every default run for a value that was then thrown
        // away by `store!`.
        //
        // The Multicall3 path below still sends it unconditionally, because
        // `calls_for_row` gets no schema and must return a fixed call count.
        // That is not free there either: `multicall_batch_size` caps *inner
        // calls*, not rows, so two calls per row halves the rows per
        // `aggregate3` (125 instead of 250) and doubles the number of
        // `aggregate3` requests. Multicall is on by default, so the default
        // path is the one paying it. Making the saving real on that path needs
        // the schema threaded into `MulticallBatchable::calls_for_row`; until
        // then, `--no-multicall` is the only way to avoid the wasted call.
        let domain_separator = if query
            .schemas
            .get_schema(&Datatype::Erc2612Nonces)
            .is_ok_and(|schema| schema.has_column("domain_separator"))
        {
            let call_data = ERC2612::DOMAIN_SEPARATORCall {}.abi_encode();
            let output = contract_read(source.call2(token, call_data, block_number).await)?;
            output.and_then(|bytes| decode_bytes32_word(&bytes))
        } else {
            None
        };

        let block = request.block_number()? as u32;
        Ok((block, request.contract()?, owner_bytes, nonce, domain_separator))
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::Erc2612Nonces)?;
        let (block, erc20, address, nonce, domain_separator) = response;
        columns.n_rows += 1;
        store!(schema, columns, block_number, block);
        store!(schema, columns, erc20, erc20);
        store!(schema, columns, address, address);
        store!(schema, columns, nonce, nonce);
        store!(schema, columns, domain_separator, domain_separator);
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

// A nonce is a state read at a block, not something a transaction produces. The
// permit that moved it is one transaction, but the value is only defined
// relative to a block, so `-t` has no row to return.
impl CollectByTransaction for Erc2612Nonces {
    type Response = ();
}

impl MulticallBatchable for Erc2612Nonces {
    fn calls_for_row(params: &Params, require_success: bool) -> R<Vec<Multicall3::Call3>> {
        let owner = params.ethers_address()?;
        let target = params.ethers_contract()?;
        // Failure is the expected answer for every token that is not an
        // ERC-2612 permit token, so `--multicall-require-success` is the only
        // thing that should ever make these calls fatal.
        let allow_failure = !require_success;
        Ok(vec![
            Multicall3::Call3 {
                target,
                allowFailure: allow_failure,
                callData: Bytes::from(ERC2612::noncesCall { owner }.abi_encode()),
            },
            Multicall3::Call3 {
                target,
                allowFailure: allow_failure,
                callData: Bytes::from(ERC2612::DOMAIN_SEPARATORCall {}.abi_encode()),
            },
        ])
    }

    fn decode_row(params: &Params, results: &[Multicall3::Result]) -> R<Self::Response> {
        // `calls_for_row` emits exactly two calls; a shorter slice means the
        // node returned a malformed aggregate3, and indexing would panic the
        // worker task rather than surface that as an error.
        let [nonce_result, domain_result] = results else {
            return Err(err("multicall returned the wrong number of results for row"))
        };
        let nonce =
            if nonce_result.success { decode_u256_word(&nonce_result.returnData) } else { None };
        let domain_separator = if domain_result.success {
            decode_bytes32_word(&domain_result.returnData)
        } else {
            None
        };
        Ok((
            params.block_number()? as u32,
            params.contract()?,
            params.address()?,
            nonce,
            domain_separator,
        ))
    }
}

/// Decode an `eth_call` return that should be a single `bytes32`.
///
/// A conformant `DOMAIN_SEPARATOR()` returns exactly one 32-byte word. Anything
/// else — an empty return (what an address with no code answers), a short
/// payload, or a longer one — is not a domain separator. Padding a short return
/// or taking the leading word of a long one would mint a separator that never
/// existed on chain, and a wrong separator reads as a valid one.
fn decode_bytes32_word(data: &[u8]) -> Option<Vec<u8>> {
    (data.len() == 32).then(|| data.to_vec())
}
