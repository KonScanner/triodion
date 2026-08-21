use crate::*;
use alloy::{
    primitives::{Bytes, FixedBytes, U256},
    sol_types::SolCall,
};
use polars::prelude::*;
use std::collections::HashMap;

/// columns for contract_interfaces
///
/// One row per (block, address): what a contract answers to ERC-165
/// `supportsInterface(bytes4)` for a fixed table of interface ids. It is the
/// classification layer the token datasets sit on — an address that answers
/// `true` for ERC-721 can be collected as an ERC-721, instead of being guessed
/// at from the shape of its logs.
///
/// Every answer is `Option<bool>` and the three states are three different
/// facts:
///
/// - `null` — the call reverted, the address has no code, or the return was not an ABI `bool`. The
///   contract said nothing, which is what a non-ERC-165 address looks like.
/// - `false` — the contract answered no.
/// - `true` — the contract answered yes.
///
/// Filling `null` with `false` would turn "never answered" into "answered no",
/// and a count of `false` would then include every EOA in the input.
///
/// `answers_true_to_everything` overrides the rest of the row — see the column.
#[triodion_macros::to_df(Datatype::ContractInterfaces)]
#[derive(Default)]
pub struct ContractInterfaces {
    n_rows: u64,
    block_number: Vec<u32>,
    address: Vec<Vec<u8>>,
    // The probe for the reserved id 0xffffffff. ERC-165 REQUIRES a compliant
    // contract to answer `false` here, so `true` means this contract answers
    // yes to every id ever asked: discard every other column in this row, none
    // of them carry information. `null` is not an all-clear either — it means
    // no decodable answer, so the contract made no ERC-165 promise about the
    // rest of the row.
    answers_true_to_everything: Vec<Option<bool>>,
    // ERC-165 requires this one to be `true`. A contract that implements
    // `supportsInterface` but answers `false` here is not ERC-165 compliant,
    // and its other answers are its own convention rather than the standard's.
    supports_erc165: Vec<Option<bool>>,
    supports_erc721: Vec<Option<bool>>,
    supports_erc721_metadata: Vec<Option<bool>>,
    supports_erc721_enumerable: Vec<Option<bool>>,
    supports_erc1155: Vec<Option<bool>>,
    supports_erc1155_metadata_uri: Vec<Option<bool>>,
    supports_erc2981: Vec<Option<bool>>,
    supports_erc1271: Vec<Option<bool>>,
    chain_id: Vec<u64>,
}

// An ERC-165 interface id is the XOR of the four-byte selectors of every
// function the interface declares, so each id below is derived rather than
// looked up. Each derivation was recomputed from keccak-256 of the signatures
// and each result matches the value published in its EIP.

/// `supportsInterface(bytes4)` 0x01ffc9a7 — a one-function interface, so the id
/// is that selector on its own.
const ID_ERC165: FixedBytes<4> = FixedBytes::new([0x01, 0xff, 0xc9, 0xa7]);

/// `balanceOf(address)` 0x70a08231 ^ `ownerOf(uint256)` 0x6352211e ^
/// `safeTransferFrom(address,address,uint256)` 0x42842e0e ^
/// `safeTransferFrom(address,address,uint256,bytes)` 0xb88d4fde ^
/// `transferFrom(address,address,uint256)` 0x23b872dd ^
/// `approve(address,uint256)` 0x095ea7b3 ^ `getApproved(uint256)` 0x081812fc ^
/// `setApprovalForAll(address,bool)` 0xa22cb465 ^
/// `isApprovedForAll(address,address)` 0xe985e9c5 = 0x80ac58cd.
const ID_ERC721: FixedBytes<4> = FixedBytes::new([0x80, 0xac, 0x58, 0xcd]);

/// `name()` 0x06fdde03 ^ `symbol()` 0x95d89b41 ^ `tokenURI(uint256)`
/// 0xc87b56dd = 0x5b5e139f.
const ID_ERC721_METADATA: FixedBytes<4> = FixedBytes::new([0x5b, 0x5e, 0x13, 0x9f]);

/// `totalSupply()` 0x18160ddd ^ `tokenOfOwnerByIndex(address,uint256)`
/// 0x2f745c59 ^ `tokenByIndex(uint256)` 0x4f6ccce7 = 0x780e9d63.
const ID_ERC721_ENUMERABLE: FixedBytes<4> = FixedBytes::new([0x78, 0x0e, 0x9d, 0x63]);

/// `balanceOf(address,uint256)` 0x00fdd58e ^
/// `balanceOfBatch(address[],uint256[])` 0x4e1273f4 ^
/// `setApprovalForAll(address,bool)` 0xa22cb465 ^
/// `isApprovedForAll(address,address)` 0xe985e9c5 ^
/// `safeTransferFrom(address,address,uint256,uint256,bytes)` 0xf242432a ^
/// `safeBatchTransferFrom(address,address,uint256[],uint256[],bytes)`
/// 0x2eb2c2d6 = 0xd9b67a26.
const ID_ERC1155: FixedBytes<4> = FixedBytes::new([0xd9, 0xb6, 0x7a, 0x26]);

/// `uri(uint256)` 0x0e89341c — a one-function interface.
const ID_ERC1155_METADATA_URI: FixedBytes<4> = FixedBytes::new([0x0e, 0x89, 0x34, 0x1c]);

/// `royaltyInfo(uint256,uint256)` 0x2a55205a — a one-function interface.
const ID_ERC2981: FixedBytes<4> = FixedBytes::new([0x2a, 0x55, 0x20, 0x5a]);

/// `isValidSignature(bytes32,bytes)` 0x1626ba7e — a one-function interface.
///
/// The superseded ERC-1271 draft took `(bytes,bytes)` and has a different id,
/// 0x20c13b0b. It is not probed, so a `null` here does not prove the contract
/// cannot verify a signature — only that it does not implement this version.
const ID_ERC1271: FixedBytes<4> = FixedBytes::new([0x16, 0x26, 0xba, 0x7e]);

/// The id ERC-165 reserves as invalid and requires every compliant contract to
/// answer `false` for. See the `answers_true_to_everything` column.
const INVALID_INTERFACE_ID: FixedBytes<4> = FixedBytes::new([0xff, 0xff, 0xff, 0xff]);

/// One `eth_call` — or one Multicall3 inner call — per probe, per row.
const N_PROBES: usize = 9;

/// Probe order. It is load-bearing: `extract`, `decode_row` and `transform`
/// all read the answers positionally, so reordering this array relabels every
/// column without changing a column name.
///
/// The invalid id goes first so that the column that invalidates the row is
/// the first one a reader meets.
const PROBES: [FixedBytes<4>; N_PROBES] = [
    INVALID_INTERFACE_ID,
    ID_ERC165,
    ID_ERC721,
    ID_ERC721_METADATA,
    ID_ERC721_ENUMERABLE,
    ID_ERC1155,
    ID_ERC1155_METADATA_URI,
    ID_ERC2981,
    ID_ERC1271,
];

/// (block_number, address, one answer per entry of [`PROBES`], in that order)
type InterfaceAnswers = (u32, Vec<u8>, [Option<bool>; N_PROBES]);

impl Dataset for ContractInterfaces {
    fn aliases() -> Vec<&'static str> {
        vec!["erc165"]
    }

    fn default_columns() -> Option<Vec<&'static str>> {
        Some(vec![
            "block_number",
            "address",
            "answers_true_to_everything",
            "supports_erc165",
            "supports_erc721",
            "supports_erc721_metadata",
            "supports_erc721_enumerable",
            "supports_erc1155",
            "supports_erc1155_metadata_uri",
            "supports_erc2981",
            "supports_erc1271",
            "chain_id",
        ])
    }

    fn default_sort() -> Option<Vec<&'static str>> {
        Some(vec!["block_number", "address"])
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

/// Gas ceiling for one `supportsInterface` probe.
///
/// ERC-165 requires the function to use at most 30,000 gas, so a conforming
/// contract never reaches this and a cap costs it nothing.
///
/// The cap is not for conforming contracts. This dataset points `eth_call` at
/// arbitrary addresses, most of which do not implement ERC-165 at all, so each
/// probe runs whatever fallback function the address happens to have — under
/// the node's own default limit, which geth sets at 50,000,000 gas. A fallback
/// that loops until the gas runs out then takes the node seconds per probe,
/// nine probes per row, and the chunk fails on a timeout instead of writing
/// one row of nulls. With the cap, the same contract produces an out-of-gas
/// revert that `contract_read` folds into `None` for that address alone.
const PROBE_GAS_LIMIT: u64 = 30_000;

impl CollectByBlock for ContractInterfaces {
    type Response = InterfaceAnswers;

    async fn extract(request: Params, source: Arc<Source>, _: Arc<Query>) -> R<Self::Response> {
        let block_number = request.ethers_block_number()?;
        let address = request.ethers_address()?;

        // Each probe folds a *contract-level* refusal (a revert, or an address
        // with no code) into `None`, while a *node-level* failure — pruned
        // state on a non-archive endpoint, a rate limit, a timeout —
        // propagates via `?`. Without that split, pointing this dataset at a
        // non-archive RPC classifies every contract as "not ERC-165" under a
        // "chunks errored: 0" banner.
        let mut answers = [None; N_PROBES];
        for (answer, interface_id) in answers.iter_mut().zip(PROBES) {
            let call_data = probe_calldata(interface_id);
            let output = contract_read(
                source
                    .call_with_gas_limit(address, call_data, block_number, Some(PROBE_GAS_LIMIT))
                    .await,
            )?;
            *answer = output.and_then(|bytes| decode_bool_word(&bytes));
        }

        Ok((request.block_number()? as u32, request.address()?, answers))
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::ContractInterfaces)?;
        let (block, address, answers) = response;
        // Positional unpack, in `PROBES` order.
        let [answers_true_to_everything, supports_erc165, supports_erc721, supports_erc721_metadata, supports_erc721_enumerable, supports_erc1155, supports_erc1155_metadata_uri, supports_erc2981, supports_erc1271] =
            answers;
        columns.n_rows += 1;
        store!(schema, columns, block_number, block);
        store!(schema, columns, address, address);
        store!(schema, columns, answers_true_to_everything, answers_true_to_everything);
        store!(schema, columns, supports_erc165, supports_erc165);
        store!(schema, columns, supports_erc721, supports_erc721);
        store!(schema, columns, supports_erc721_metadata, supports_erc721_metadata);
        store!(schema, columns, supports_erc721_enumerable, supports_erc721_enumerable);
        store!(schema, columns, supports_erc1155, supports_erc1155);
        store!(schema, columns, supports_erc1155_metadata_uri, supports_erc1155_metadata_uri);
        store!(schema, columns, supports_erc2981, supports_erc2981);
        store!(schema, columns, supports_erc1271, supports_erc1271);
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

// The interfaces a contract declares are a property of its code at a block, not
// of any transaction. `-t` has no relationship to resolve here: it could only
// mean "the interfaces of some address this transaction happened to touch",
// which is a different question from the one this dataset answers.
impl CollectByTransaction for ContractInterfaces {
    type Response = ();
}

impl MulticallBatchable for ContractInterfaces {
    fn calls_for_row(params: &Params, require_success: bool) -> R<Vec<Multicall3::Call3>> {
        let target = params.ethers_address()?;
        let allow_failure = !require_success;
        Ok(PROBES
            .into_iter()
            .map(|interface_id| Multicall3::Call3 {
                target,
                allowFailure: allow_failure,
                callData: Bytes::from(probe_calldata(interface_id)),
            })
            .collect())
    }

    fn default_multicall_batch_size() -> u32 {
        // 90 inner calls, i.e. 10 rows of 9 probes, against the crate default
        // of 250 (27 rows).
        //
        // `aggregate3` hands each inner call all the gas remaining at that
        // point, and this dataset aims its probes at arbitrary addresses whose
        // fallback functions are arbitrary code. One address whose fallback
        // burns the whole `eth_call` budget starves every later call in the
        // same batch, and a starved inner call returns `success: false` with
        // empty `returnData` — byte-identical to a revert. `aggregate3` still
        // returns normally, so those rows are published as nulls with nothing
        // reported.
        //
        // A smaller batch does not prevent that; nothing available in
        // `aggregate3` can, because `Call3` has no per-call gas field. It
        // bounds the damage to nine other rows instead of twenty-six. The
        // per-call path caps each probe at `PROBE_GAS_LIMIT` and does not have
        // the problem, so `--no-multicall` is the way to rule it out entirely.
        90
    }

    fn decode_row(params: &Params, results: &[Multicall3::Result]) -> R<Self::Response> {
        // `calls_for_row` emits exactly `N_PROBES` calls. A shorter slice means
        // the node returned a malformed aggregate3; zipping it would leave the
        // trailing columns null and silently publish a partial row.
        if results.len() != N_PROBES {
            return Err(err("multicall returned the wrong number of results for row"))
        }
        let mut answers = [None; N_PROBES];
        for (answer, result) in answers.iter_mut().zip(results.iter()) {
            // A failed inner call is the contract declining to answer, exactly
            // as a revert is on the per-call path.
            *answer = if result.success { decode_bool_word(&result.returnData) } else { None };
        }
        Ok((params.block_number()? as u32, params.address()?, answers))
    }
}

/// Calldata for one `supportsInterface(interface_id)` probe.
///
/// Encoded through the ABI rather than by hand: `bytes4` is left-aligned in its
/// word, and hand-padding it on the wrong side asks about a different id.
fn probe_calldata(interface_id: FixedBytes<4>) -> Vec<u8> {
    ERC165::supportsInterfaceCall { interfaceId: interface_id }.abi_encode()
}

/// Decode an `eth_call` return that should be a single ABI `bool`.
///
/// Solidity's own decoder accepts only the two canonical words and reverts on
/// anything else, so an on-chain caller of `supportsInterface` never sees a
/// third value. Mirroring that is the only reading that invents nothing: an
/// empty return (what an address with no code answers), a short or long
/// return, and a dirty word all mean the contract did not answer — `None`, not
/// `false`. It also means a contract that returns garbage for every id lands
/// with `answers_true_to_everything` null rather than false, so nothing in the
/// row claims a standard.
fn decode_bool_word(data: &[u8]) -> Option<bool> {
    // Exact-width check lives in `decode_u256_word`: a 1-byte or 33-byte return
    // is not a `bool` answer.
    let word = decode_u256_word(data)?;
    if word == U256::ZERO {
        Some(false)
    } else if word == U256::from(1u8) {
        Some(true)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_two_canonical_words_are_an_answer() {
        let mut yes = [0u8; 32];
        yes[31] = 1;
        assert_eq!(decode_bool_word(&yes), Some(true));
        assert_eq!(decode_bool_word(&[0u8; 32]), Some(false));

        // An address with no code answers with an empty return. Reading that
        // as `false` would claim the contract said no.
        assert_eq!(decode_bool_word(&[]), None);

        // Dirty words: Solidity's decoder reverts on both, so neither is an
        // answer a caller could have acted on.
        let mut two = [0u8; 32];
        two[31] = 2;
        assert_eq!(decode_bool_word(&two), None);
        let mut high_bit_set = [0u8; 32];
        high_bit_set[0] = 1;
        high_bit_set[31] = 1;
        assert_eq!(decode_bool_word(&high_bit_set), None);
    }

    #[test]
    fn every_probe_is_distinct() {
        let mut ids = PROBES.to_vec();
        ids.sort();
        ids.dedup();
        // A duplicated id would label two columns from one question, and the
        // duplicate column would look like independent evidence.
        assert_eq!(ids.len(), N_PROBES);
    }

    #[test]
    fn the_invalid_id_is_the_reserved_one() {
        // The whole point of the sentinel column: ask for the id ERC-165
        // reserves, not for a real interface.
        assert_eq!(INVALID_INTERFACE_ID, FixedBytes::new([0xff; 4]));
        assert_eq!(PROBES.first(), Some(&INVALID_INTERFACE_ID));
    }
}
