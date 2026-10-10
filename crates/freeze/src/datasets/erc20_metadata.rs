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
        // The calls of the Multicall3 path below, sent one at a time. A revert,
        // or an address with no code, becomes a null; a node that could not
        // serve the state propagates, so the chunk is counted as errored
        // rather than written out as nulls.
        extract_by_eth_call::<Self>(request, source).await
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
        Ok((u32::try_from(params.block_number()?)?, params.address()?, name, symbol, decimals))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::{
        providers::ProviderBuilder, rpc::json_rpc::ErrorPayload, sol_types::SolValue,
        transports::mock::Asserter,
    };
    use std::borrow::Cow;

    #[tokio::test]
    async fn the_per_call_path_keeps_each_answer_in_its_own_column() {
        let asserter = Asserter::new();
        // name, then symbol (reverts), then decimals.
        asserter.push_success(&Bytes::from(String::from("Token").abi_encode()));
        asserter.push_failure(ErrorPayload {
            code: 3,
            message: Cow::Borrowed("execution reverted"),
            data: None,
        });
        let mut decimals = [0u8; 32];
        decimals[31] = 18;
        asserter.push_success(&Bytes::from(decimals.to_vec()));
        let provider = ProviderBuilder::default().connect_mocked_client(asserter);
        let source =
            Arc::new(Source::from_provider(provider, 1, &SourceConfig::new(String::new())));
        let params =
            Params { block_number: Some(1), address: Some(vec![0x11; 20]), ..Default::default() };

        let (block, _, name, symbol, decimals) =
            extract_by_eth_call::<Erc20Metadata>(params, source).await.expect("one revert is data");

        assert_eq!(block, 1);
        assert_eq!(name.as_deref(), Some("Token"));
        assert_eq!(symbol, None, "the revert becomes a null");
        assert_eq!(decimals, Some(18));
    }
}
