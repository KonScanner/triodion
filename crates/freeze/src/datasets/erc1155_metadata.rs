use crate::{
    types::rpc_params::{fixed_from_slice, log_address_matches, topic_matches},
    *,
};
use alloy::{
    primitives::{B256, U256},
    rpc::types::{Filter, Log, Topic},
    sol_types::SolEvent,
};
use polars::prelude::*;

/// The ERC-1155 metadata substitution token. A conforming client replaces this
/// exact lowercase literal with the token id as 64 lowercase hex digits and no
/// `0x` prefix. Matching case-insensitively would flag `{ID}` strings that no
/// conforming client expands, so the match stays literal.
const ID_PLACEHOLDER: &str = "{id}";

/// columns for erc1155_metadata
///
/// One row per ERC-1155 `URI(string value, uint256 indexed id)` log.
///
/// An empty result is the normal case here, not a failure. The event is
/// optional in the standard, and most contracts never emit it: they serve one
/// static `{id}` template from `uri(id)` and never announce a change. A
/// full-range scan of a busy chain can legitimately return zero rows.
///
/// This reads logs instead of calling `uri(id)`. A call needs a token id to
/// call with, and triodion partitions by block, address and topic — there is no
/// token-id dimension to enumerate ids from, so the call form has no input.
#[triodion_macros::to_df(Datatype::Erc1155Metadata)]
#[derive(Default)]
pub struct Erc1155Metadata {
    n_rows: u64,
    block_number: Vec<u32>,
    block_hash: Vec<Option<Vec<u8>>>,
    transaction_index: Vec<u32>,
    log_index: Vec<u32>,
    transaction_hash: Vec<Vec<u8>>,
    erc1155: Vec<Vec<u8>>,
    // Indexed, so it arrives in topics[1] rather than in the log data.
    token_id: Vec<U256>,
    // The event's `value` argument. It is not indexed, so it sits in the log
    // data as an ABI-encoded dynamic string and has to be decoded, not sliced.
    uri: Vec<String>,
    // Derived, not reported: `uri` contains the literal `{id}`. Such a string
    // is a template, not a URL, and fetching it verbatim fails. Expand it with
    // this row's `token_id` first. It also means the string is shared by every
    // token of the contract, so it does not identify this token.
    is_uri_template: Vec<bool>,
    // True when the on-chain bytes were not valid UTF-8 and `uri` therefore
    // holds U+FFFD replacement characters that were never on chain. Without
    // this the mangled string is indistinguishable from a faithful one.
    is_uri_lossy: Vec<bool>,
    chain_id: Vec<u64>,
}

impl Dataset for Erc1155Metadata {
    fn aliases() -> Vec<&'static str> {
        vec!["erc1155_uris"]
    }

    fn default_columns() -> Option<Vec<&'static str>> {
        Some(vec![
            "block_number",
            // "block_hash",
            "transaction_index",
            "log_index",
            "transaction_hash",
            "erc1155",
            "token_id",
            "uri",
            "is_uri_template",
            "is_uri_lossy",
            "chain_id",
        ])
    }

    fn optional_parameters() -> Vec<Dim> {
        // Topic0 is pinned to the URI signature. Topic1 is the indexed token
        // id, so `--topic1` narrows to one token. The event has one indexed
        // argument, so topic2 and topic3 do not exist on it.
        vec![Dim::Address, Dim::Topic1]
    }

    fn use_block_ranges() -> bool {
        true
    }

    fn default_inner_request_size() -> u64 {
        // Same as the other log datasets. A wider window would still return
        // few rows, since URI logs are rare, but the dispatcher takes
        // max(user, default): raising this would stop a user shrinking the
        // window for a provider that caps the block span of eth_getLogs.
        50
    }

    fn arg_aliases() -> Option<std::collections::HashMap<Dim, Dim>> {
        Some([(Dim::Contract, Dim::Address)].into_iter().collect())
    }
}

impl CollectByBlock for Erc1155Metadata {
    type Response = Vec<Log>;

    async fn extract(request: Params, source: Arc<Source>, _: Arc<Query>) -> R<Self::Response> {
        let mut topics: [Topic; 4] = Default::default();
        topics[0] = ERC1155::URI::SIGNATURE_HASH.into();
        if let Some(token_id) = &request.topic1 {
            // `--topic1` is hex-decoded without a width check, so a short value
            // must become an error here rather than panic in `B256::from_slice`.
            topics[1] = fixed_from_slice::<B256>(token_id, "topic1")?.into();
        }
        let filter = Filter { topics, ..request.ethers_log_filter()? };
        let logs = source.get_logs(&filter).await?;
        Ok(logs.into_iter().filter(is_erc1155_uri).collect())
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::Erc1155Metadata)?;
        process_erc1155_uris(response, columns, schema)
    }
}

impl CollectByTransaction for Erc1155Metadata {
    type Response = Vec<Log>;

    async fn extract(request: Params, source: Arc<Source>, _: Arc<Query>) -> R<Self::Response> {
        let logs = source.get_transaction_logs(request.transaction_hash()?).await?;
        // The dims never reach the node on this path: `--txs` asks for one
        // transaction's whole receipt, so the narrowing the by-block path pins
        // into the `eth_getLogs` filter has to be re-applied here. Without it a
        // dim is accepted, counted into the partition set, printed in the run
        // summary, and then ignored — the run returns rows it was asked to
        // exclude and says nothing.
        Ok(logs
            .into_iter()
            .filter(|log| {
                is_erc1155_uri(log) &&
                    log_address_matches(log, &request.address) &&
                    topic_matches(log, 1, &request.topic1)
            })
            .collect())
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::Erc1155Metadata)?;
        process_erc1155_uris(response, columns, schema)
    }
}

/// True iff `log` has the ERC-1155 `URI` shape: the URI signature and exactly 2
/// topics (sig + indexed id). The data width is not checked here — the string
/// body is dynamic, so only the ABI decode can judge it.
///
/// The per-transaction path needs the signature check because it receives every
/// log of the transaction, unfiltered.
pub(crate) fn is_erc1155_uri(log: &Log) -> bool {
    log.topics().len() == 2 &&
        log.topics().first().is_some_and(|t| *t == ERC1155::URI::SIGNATURE_HASH)
}

/// process logs into columns
fn process_erc1155_uris(logs: Vec<Log>, columns: &mut Erc1155Metadata, schema: &Table) -> R<()> {
    for log in logs.iter() {
        // Shape guard first: a log carrying the URI topic0 with fewer than 2
        // topics would otherwise reach the decode below with a missing id.
        // Every current caller pre-filters; this keeps a future caller that
        // arrives with mixed logs safe.
        if !is_erc1155_uri(log) {
            continue;
        }
        if let (Some(bn), Some(tx), Some(ti), Some(li)) =
            (log.block_number, log.transaction_hash, log.transaction_index, log.log_index)
        {
            // Reads topics[1] for the indexed `id` and ABI-decodes the data for
            // the non-indexed `value`. `decode_log_data` detokenises a `string`
            // with `String::from_utf8_lossy`, so non-UTF-8 bytes would silently
            // become U+FFFD and be written as if measured. Validate first, and
            // fall back to the lossy read only to keep the row: dropping it
            // would lose the fact that the URI changed, which is the signal
            // this dataset exists for. A truncated body or a bad offset still
            // fails both decoders and skips the row.
            let (event, is_lossy) = match ERC1155::URI::decode_log_data_validate(log.data()) {
                Ok(event) => (event, false),
                Err(_) => match ERC1155::URI::decode_log_data(log.data()) {
                    Ok(event) => (event, true),
                    Err(_) => continue,
                },
            };
            let is_template = event.value.contains(ID_PLACEHOLDER);

            columns.n_rows += 1;
            store!(schema, columns, block_number, bn as u32);
            store!(schema, columns, block_hash, log.block_hash.map(|bh| bh.to_vec()));
            store!(schema, columns, transaction_index, ti as u32);
            store!(schema, columns, log_index, li as u32);
            store!(schema, columns, transaction_hash, tx.to_vec());
            store!(schema, columns, erc1155, log.address().to_vec());
            store!(schema, columns, token_id, event.id);
            store!(schema, columns, uri, event.value);
            store!(schema, columns, is_uri_template, is_template);
            store!(schema, columns, is_uri_lossy, is_lossy);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::Address;

    /// real `Erc1155Metadata` schema, so `store!` actually evaluates each column
    fn schema() -> Table {
        Datatype::Erc1155Metadata
            .table_schema(
                &[U256Type::String],
                &ColumnEncoding::Hex,
                &None,
                &None,
                &None,
                None,
                None,
            )
            .expect("Erc1155Metadata has a valid default schema")
    }

    /// rpc `Log` carrying the block/tx/index fields `process_erc1155_uris` needs
    fn log_from(data: alloy::primitives::LogData) -> Log {
        Log {
            inner: alloy::primitives::Log { address: Address::ZERO, data },
            block_number: Some(1),
            transaction_hash: Some(B256::ZERO),
            transaction_index: Some(0),
            log_index: Some(0),
            ..Default::default()
        }
    }

    /// a well-formed URI log, encoded the way a contract would emit it
    fn uri_log(value: &str, id: u64) -> Log {
        log_from(ERC1155::URI { value: value.to_string(), id: U256::from(id) }.encode_log_data())
    }

    #[test]
    fn decodes_the_indexed_id_and_the_non_indexed_string() {
        // The point of the dataset: `id` is a topic, `value` is in the data.
        let log = uri_log("ipfs://QmExample/7.json", 7);
        let mut columns = Erc1155Metadata::default();
        process_erc1155_uris(vec![log], &mut columns, &schema()).unwrap();

        assert_eq!(columns.n_rows, 1);
        assert_eq!(columns.token_id, vec![U256::from(7u64)]);
        assert_eq!(columns.uri, vec!["ipfs://QmExample/7.json".to_string()]);
        assert_eq!(columns.is_uri_template, vec![false]);
    }

    #[test]
    fn flags_a_template_uri() {
        let log = uri_log("https://example.com/api/{id}.json", 1);
        let mut columns = Erc1155Metadata::default();
        process_erc1155_uris(vec![log], &mut columns, &schema()).unwrap();
        assert_eq!(columns.is_uri_template, vec![true]);
    }

    #[test]
    fn does_not_flag_a_non_conforming_placeholder() {
        // `{ID}` is not the spec's token, so no client expands it. Flagging it
        // would claim a substitution that never happens.
        let log = uri_log("https://example.com/api/{ID}.json", 1);
        let mut columns = Erc1155Metadata::default();
        process_erc1155_uris(vec![log], &mut columns, &schema()).unwrap();
        assert_eq!(columns.is_uri_template, vec![false]);
    }

    #[test]
    fn keeps_an_empty_uri_as_a_value() {
        // A contract can emit `URI("", id)` to clear a token's metadata. That
        // is a measurement, not an absence, so the row exists and holds "".
        let log = uri_log("", 3);
        let mut columns = Erc1155Metadata::default();
        process_erc1155_uris(vec![log], &mut columns, &schema()).unwrap();
        assert_eq!(columns.n_rows, 1);
        assert_eq!(columns.uri, vec![String::new()]);
    }

    #[test]
    fn uri_topic0_with_missing_topic1_is_skipped_not_panicking() {
        let mut data = ERC1155::URI { value: "x".to_string(), id: U256::ZERO }.encode_log_data();
        data.set_topics_unchecked(vec![ERC1155::URI::SIGNATURE_HASH]);
        let malformed = log_from(data);
        assert!(!is_erc1155_uri(&malformed));

        let mut columns = Erc1155Metadata::default();
        process_erc1155_uris(vec![malformed], &mut columns, &schema()).unwrap();
        assert_eq!(columns.n_rows, 0, "malformed log must not produce a row");
    }

    #[test]
    fn undecodable_data_is_skipped() {
        // Right topics, but the body is not an ABI-encoded string.
        let data = alloy::primitives::LogData::new_unchecked(
            vec![ERC1155::URI::SIGNATURE_HASH, B256::ZERO],
            alloy::primitives::Bytes::from(vec![0xffu8; 16]),
        );
        let mut columns = Erc1155Metadata::default();
        process_erc1155_uris(vec![log_from(data)], &mut columns, &schema()).unwrap();
        assert_eq!(columns.n_rows, 0);
    }

    #[test]
    fn skips_a_log_with_a_foreign_topic0() {
        let data = alloy::primitives::LogData::new_unchecked(
            vec![B256::repeat_byte(0xfe), B256::ZERO],
            alloy::primitives::Bytes::new(),
        );
        let logs = vec![log_from(data), uri_log("ipfs://a", 1)];
        let mut columns = Erc1155Metadata::default();
        process_erc1155_uris(logs, &mut columns, &schema()).unwrap();
        assert_eq!(columns.n_rows, 1);
    }

    #[test]
    fn skips_logs_missing_identity_fields() {
        let mut log = uri_log("ipfs://a", 1);
        log.log_index = None;
        let mut columns = Erc1155Metadata::default();
        process_erc1155_uris(vec![log], &mut columns, &schema()).unwrap();
        assert_eq!(columns.n_rows, 0);
    }
}
