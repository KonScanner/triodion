use crate::*;
use alloy::{
    primitives::{Bytes, U256},
    sol_types::SolCall,
};
use polars::prelude::*;
use std::collections::HashMap;

/// columns for erc4626_metadata
///
/// One row per (vault, block): the three reads that describe an ERC-4626 vault
/// at that block.
///
/// There is deliberately no share-price column. A price per share is
/// `total_assets / total_supply`, and both operands are already in the row, so
/// the column would add no information while forcing three choices on every
/// reader: what to do when `total_supply` is 0 (a fresh vault, or one whose
/// shares were all redeemed — the ratio is undefined, not 1.0), how many
/// decimals to round to, and which of the two token decimals to scale by. Any
/// answer picked here is wrong for someone. `convertToAssets(1e18)` is also not
/// the same number for vaults that charge an exit fee, so a single column
/// cannot even name one convention. Consumers divide the two columns
/// themselves, with their own convention.
#[triodion_macros::to_df(Datatype::Erc4626Metadata)]
#[derive(Default)]
pub struct Erc4626Metadata {
    n_rows: u64,
    block_number: Vec<u32>,
    // The vault, i.e. the share token. `asset` below is the token it holds.
    erc4626: Vec<Vec<u8>>,
    asset: Vec<Option<Vec<u8>>>,
    // Null means the vault refused the read (a revert, or no code at that
    // address at that block). Zero means the vault answered zero, which is a
    // real state: a vault that has just been deployed, or fully drained.
    total_assets: Vec<Option<U256>>,
    // Shares outstanding, not the supply of the underlying token. Zero here is
    // real on a fresh vault; with a non-zero `total_assets` it is the donated
    // assets state that the inflation attack exploits. Never rewrite it as a
    // null.
    total_supply: Vec<Option<U256>>,
    chain_id: Vec<u64>,
}

impl Dataset for Erc4626Metadata {
    fn default_columns() -> Option<Vec<&'static str>> {
        Some(vec!["block_number", "erc4626", "asset", "total_assets", "total_supply", "chain_id"])
    }

    fn default_sort() -> Option<Vec<&'static str>> {
        Some(vec!["erc4626", "block_number"])
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

impl CollectByBlock for Erc4626Metadata {
    type Response = (u32, Vec<u8>, Option<Vec<u8>>, Option<U256>, Option<U256>);

    async fn extract(request: Params, source: Arc<Source>, _: Arc<Query>) -> R<Self::Response> {
        let block_number = request.ethers_block_number()?;
        let address = request.ethers_address()?;

        // Each read folds a *contract-level* refusal (revert, or an address
        // with no code) into `None`, while a *node-level* failure — pruned
        // state on a non-archive endpoint, a rate limit, a timeout —
        // propagates via `?`. Without that split, pointing this dataset at a
        // non-archive RPC yields a file of nulls under a "chunks errored: 0"
        // banner.

        // asset
        let call_data = ERC4626::assetCall {}.abi_encode();
        let asset = contract_read(source.call2(address, call_data, block_number).await)?
            .and_then(|output| decode_address_word(&output));

        // totalAssets
        let call_data = ERC4626::totalAssetsCall {}.abi_encode();
        let total_assets = contract_read(source.call2(address, call_data, block_number).await)?
            .and_then(|output| decode_u256_word(&output));

        // totalSupply
        let call_data = ERC4626::totalSupplyCall {}.abi_encode();
        let total_supply = contract_read(source.call2(address, call_data, block_number).await)?
            .and_then(|output| decode_u256_word(&output));

        Ok((request.block_number()? as u32, request.address()?, asset, total_assets, total_supply))
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::Erc4626Metadata)?;
        let (block, vault, asset, total_assets, total_supply) = response;
        columns.n_rows += 1;
        store!(schema, columns, block_number, block);
        store!(schema, columns, erc4626, vault);
        store!(schema, columns, asset, asset);
        store!(schema, columns, total_assets, total_assets);
        store!(schema, columns, total_supply, total_supply);
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

impl CollectByTransaction for Erc4626Metadata {
    type Response = ();
}

impl MulticallBatchable for Erc4626Metadata {
    fn calls_for_row(params: &Params, require_success: bool) -> R<Vec<Multicall3::Call3>> {
        let target = params.ethers_address()?;
        let allow_failure = !require_success;
        // Same encodings as the per-call path above, so both paths send the
        // same bytes for the same row.
        Ok(vec![
            Multicall3::Call3 {
                target,
                allowFailure: allow_failure,
                callData: Bytes::from(ERC4626::assetCall {}.abi_encode()),
            },
            Multicall3::Call3 {
                target,
                allowFailure: allow_failure,
                callData: Bytes::from(ERC4626::totalAssetsCall {}.abi_encode()),
            },
            Multicall3::Call3 {
                target,
                allowFailure: allow_failure,
                callData: Bytes::from(ERC4626::totalSupplyCall {}.abi_encode()),
            },
        ])
    }

    fn default_multicall_batch_size() -> u32 {
        // 60 inner calls, i.e. 20 rows of 3, against the crate default of 250
        // (83 rows).
        //
        // `totalAssets()` is the expensive inner call the default's own doc
        // says to override for. On a vault that iterates strategies it is a
        // six-figure-gas read, so 83 rows of it can approach geth's 50,000,000
        // `eth_call` cap — and a batch that runs out of gas does not error:
        // the starved inner calls return `success: false` with empty
        // `returnData`, which this dataset maps to nulls.
        60
    }

    fn decode_row(params: &Params, results: &[Multicall3::Result]) -> R<Self::Response> {
        // `calls_for_row` emits exactly three calls; a shorter slice means the
        // node returned a malformed aggregate3, and indexing would panic the
        // worker task rather than surface that as an error.
        let [asset_result, total_assets_result, total_supply_result] = results else {
            return Err(err("multicall returned the wrong number of results for row"))
        };
        let asset =
            if asset_result.success { decode_address_word(&asset_result.returnData) } else { None };
        let total_assets = if total_assets_result.success {
            decode_u256_word(&total_assets_result.returnData)
        } else {
            None
        };
        let total_supply = if total_supply_result.success {
            decode_u256_word(&total_supply_result.returnData)
        } else {
            None
        };
        Ok((params.block_number()? as u32, params.address()?, asset, total_assets, total_supply))
    }
}

/// Decode an `eth_call` return that should be a single `address`.
///
/// A conformant `asset()` returns exactly one 32-byte word whose upper 12
/// bytes are zero. Nothing else is an address:
///
/// - an empty return is what an address with no code answers, and taking its low 20 bytes is not
///   possible at all;
/// - a word with dirty upper bytes is a packed storage slot or some other payload, and masking it
///   down to 20 bytes invents an underlying token the vault never named.
///
/// Both alternatives fabricate an address, so the strict width-and-padding
/// check is the only reading that cannot.
fn decode_address_word(data: &[u8]) -> Option<Vec<u8>> {
    // The length test runs first, so the two slices are always in bounds.
    (data.len() == 32 && data[..12].iter().all(|b| *b == 0)).then(|| data[12..].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_padded_word_decodes_to_the_low_twenty_bytes() {
        let mut word = [0u8; 32];
        word[12..].copy_from_slice(&[0xab; 20]);
        assert_eq!(decode_address_word(&word), Some(vec![0xab; 20]));
    }

    #[test]
    fn an_empty_return_is_not_an_address() {
        // What an EOA, or a self-destructed vault, answers.
        assert_eq!(decode_address_word(&[]), None);
    }

    #[test]
    fn a_word_with_dirty_upper_bytes_is_not_an_address() {
        let mut word = [0u8; 32];
        word[0] = 1;
        word[12..].copy_from_slice(&[0xab; 20]);
        assert_eq!(decode_address_word(&word), None, "masking a packed slot invents an address");
    }
}
