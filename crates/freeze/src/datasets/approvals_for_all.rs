use crate::{
    types::rpc_params::{
        address_topic_matches, fixed_from_slice, log_address_matches, topic_matches,
    },
    *,
};
use alloy::{
    primitives::B256,
    rpc::types::{Filter, Log, Topic},
    sol_types::SolEvent,
};
use polars::prelude::*;

/// columns for `ApprovalForAll(address,address,bool)` logs.
///
/// ERC-721 and ERC-1155 declare this event with the same argument types, so
/// both hash to one topic0. A log that carries it does not say which of the two
/// standards the emitting contract implements, and no decoding can recover
/// that: the bytes are identical. This is why the dataset is not called
/// `erc1155_approvals` — that name would be false for about half of its rows.
/// To classify a contract, join `contract_address` against the
/// `contract_interfaces` dataset, which asks the contract itself through the
/// ERC-165 `supportsInterface` call.
///
/// `approved == false` is a revocation. It is a measurement of what happened,
/// not an absence of one, so it is stored as `false` and never as null.
///
/// These rows are an event log, not a state table. `WHERE approved` on its own
/// does not give the live approvals: an operator approved in one block and
/// revoked in the next leaves both rows behind, and the filter keeps the stale
/// one. First take the row with the largest `(block_number, log_index)` for
/// each `(contract_address, owner, operator)`, then filter on `approved`.
#[triodion_macros::to_df(Datatype::ApprovalsForAll)]
#[derive(Default)]
pub struct ApprovalsForAll {
    n_rows: u64,
    block_number: Vec<u32>,
    block_hash: Vec<Option<Vec<u8>>>,
    transaction_index: Vec<u32>,
    log_index: Vec<u32>,
    transaction_hash: Vec<Vec<u8>>,
    // The token contract that emitted the log. Which standard it implements is
    // not recorded anywhere in the log; see the note above the struct.
    contract_address: Vec<Vec<u8>>,
    // The token holder whose permission changes. ERC-1155 names this argument
    // `account` and ERC-721 names it `owner`; it is the same topic either way.
    owner: Vec<Vec<u8>>,
    // The address that gains or loses permission over every token the owner
    // holds in this contract. Unlike the ERC-721 `Approval` event, this grant
    // names no token id, so it also covers tokens acquired after this block.
    operator: Vec<Vec<u8>>,
    // `true` grants, `false` revokes. Both are events that happened.
    approved: Vec<bool>,
    chain_id: Vec<u64>,
}

impl Dataset for ApprovalsForAll {
    fn aliases() -> Vec<&'static str> {
        // Both names arrive at the same rows, because both standards emit the
        // same topic0. Neither name is used for the dataset itself, so that no
        // output file claims a standard the logs cannot prove.
        vec!["erc721_approvals_for_all", "erc1155_approvals"]
    }

    fn default_columns() -> Option<Vec<&'static str>> {
        Some(vec![
            "block_number",
            // "block_hash",
            "transaction_index",
            "log_index",
            "transaction_hash",
            "contract_address",
            "owner",
            "operator",
            "approved",
            "chain_id",
        ])
    }

    fn optional_parameters() -> Vec<Dim> {
        // Topic0 is fixed by the definition of this dataset, so it is not
        // offered. Topic1 is the owner and Topic2 is the operator, and both are
        // reachable two ways: as a raw 32-byte topic, or as an ordinary 20-byte
        // address via `--from-address` / `--to-address`, which is what
        // `erc20_approvals` calls its owner and spender. The names are loose —
        // an approval has no direction — but demanding a hand-padded 32-byte
        // word for a value the user reads off a block explorer is worse.
        // Unlike the multi-event log datasets, this dataset carries exactly one
        // event signature, so topic1 and topic2 mean the same thing on every
        // row and address-shaped filtering is unambiguous here.
        vec![Dim::Address, Dim::Topic1, Dim::Topic2, Dim::FromAddress, Dim::ToAddress]
    }

    fn use_block_ranges() -> bool {
        true
    }

    fn default_inner_request_size() -> u64 {
        // Log fetch is one HTTP request per block range. 50 blocks per request
        // matches the other log datasets; ApprovalForAll is far rarer per block
        // than Transfer, so the response stays small. Override with
        // --inner-request-size.
        50
    }

    fn arg_aliases() -> Option<std::collections::HashMap<Dim, Dim>> {
        Some([(Dim::Contract, Dim::Address)].into_iter().collect())
    }
}

impl CollectByBlock for ApprovalsForAll {
    type Response = Vec<Log>;

    async fn extract(request: Params, source: Arc<Source>, _: Arc<Query>) -> R<Self::Response> {
        let mut topics: [Topic; 4] = Default::default();
        topics[0] = APPROVAL_FOR_ALL_TOPIC0.into();

        // Each slot is reachable two ways, and supplying both is refused
        // rather than resolved by precedence. Preferring one silently discards
        // the other, and the discarded dimension does not go away: it is still
        // a partition dimension, so the run is still multiplied by it and
        // emits N identically-named-apart files holding identical rows from
        // one identical `eth_getLogs`. An error costs the user one flag; the
        // alternative costs them N copies of the same data.
        if request.topic1.is_some() && request.from_address.is_some() {
            return Err(err("--topic1 and --from-address both select topic1; give one, not both"))
        }
        if request.topic2.is_some() && request.to_address.is_some() {
            return Err(err("--topic2 and --to-address both select topic2; give one, not both"))
        }

        if let Some(owner) = &request.topic1 {
            topics[1] = fixed_from_slice::<B256>(owner, "topic1")?.into();
        } else if let Some(owner) = &request.from_address {
            let v = address_dim_as_topic(owner)
                .ok_or_else(|| err("from_address must be at most 32 bytes"))?;
            topics[1] = v.into();
        }
        if let Some(operator) = &request.topic2 {
            topics[2] = fixed_from_slice::<B256>(operator, "topic2")?.into();
        } else if let Some(operator) = &request.to_address {
            let v = address_dim_as_topic(operator)
                .ok_or_else(|| err("to_address must be at most 32 bytes"))?;
            topics[2] = v.into();
        }
        let filter = Filter { topics, ..request.ethers_log_filter()? };
        let logs = source.get_logs(&filter).await?;
        Ok(logs.into_iter().filter(is_approval_for_all_shape).collect())
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::ApprovalsForAll)?;
        process_approvals_for_all(response, columns, schema)
    }
}

impl CollectByTransaction for ApprovalsForAll {
    type Response = Vec<Log>;

    async fn extract(request: Params, source: Arc<Source>, _: Arc<Query>) -> R<Self::Response> {
        let logs = source.get_transaction_logs(request.transaction_hash()?).await?;
        // The dims never reach the node on this path: `--txs` asks for one
        // transaction's whole receipt, so the narrowing the by-block path pins
        // into the `eth_getLogs` filter has to be re-applied here. Without it a
        // dim is accepted, counted into the partition set, printed in the run
        // summary, and then ignored — the run returns rows it was asked to
        // exclude and says nothing.
        //
        // Each slot is reachable two ways, and the by-block path lets the raw
        // topic win, so this one does too. It cannot be reached with both set:
        // `CollectByBlock::extract` refuses that combination.
        let owner_matches = |log: &Log| match &request.topic1 {
            Some(_) => topic_matches(log, 1, &request.topic1),
            None => address_topic_matches(log, 1, &request.from_address),
        };
        let operator_matches = |log: &Log| match &request.topic2 {
            Some(_) => topic_matches(log, 2, &request.topic2),
            None => address_topic_matches(log, 2, &request.to_address),
        };
        Ok(logs
            .into_iter()
            .filter(|log| {
                is_approval_for_all(log) &&
                    log_address_matches(log, &request.address) &&
                    owner_matches(log) &&
                    operator_matches(log)
            })
            .collect())
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::ApprovalsForAll)?;
        process_approvals_for_all(response, columns, schema)
    }
}

/// topic0 of `ApprovalForAll(address,address,bool)`.
///
/// Taken from the ERC-721 declaration, but the ERC-1155 declaration produces
/// the same 32 bytes, and one filter therefore returns the logs of both. The
/// test below holds the two constants equal so that a future edit to either
/// `sol!` block cannot make this dataset silently drop half of its rows.
const APPROVAL_FOR_ALL_TOPIC0: B256 = ERC721::ApprovalForAll::SIGNATURE_HASH;

/// True iff `log` has the `ApprovalForAll` shape: 3 topics (signature, indexed
/// owner, indexed operator) and a single 32-byte word of data.
fn is_approval_for_all_shape(log: &Log) -> bool {
    log.topics().len() == 3 && log.data().data.len() == 32
}

/// True iff `log` is an `ApprovalForAll`: the shape check plus a topic0 match.
/// The per-transaction path needs the topic0 check because, unlike the by-block
/// path, `eth_getLogs` did not pre-filter the signature — we hold every log of
/// the transaction and must pick these out. Written to stay reusable by a
/// coalesced [`crate::LogEvents`] fan-out, which this datatype is NOT a member
/// of today (see `MultiDatatype::LogEvents`).
pub(crate) fn is_approval_for_all(log: &Log) -> bool {
    is_approval_for_all_shape(log) &&
        log.topics().first().is_some_and(|t| *t == APPROVAL_FOR_ALL_TOPIC0)
}

/// Read an ABI-encoded bool out of its 32-byte word.
///
/// Solidity writes 0 or 1, but a log built by hand in assembly is under no such
/// rule, and a dirty word must not read as `false`. Judge it the way the EVM
/// judges truth, with `ISZERO`: any non-zero word is `true`.
fn decode_bool_word(word: &[u8]) -> bool {
    word.iter().any(|byte| *byte != 0)
}

/// process logs into columns
fn process_approvals_for_all(
    logs: Vec<Log>,
    columns: &mut ApprovalsForAll,
    schema: &Table,
) -> R<()> {
    for log in logs.iter() {
        // Shape guard first. The columns below index topics()[1] and [2], so a
        // log with this topic0 but fewer topics would panic the worker task.
        // Every current caller pre-filters; this keeps a future caller that
        // arrives with mixed logs safe, and skips such a log instead.
        if !is_approval_for_all(log) {
            continue;
        }
        if let (Some(bn), Some(tx), Some(ti), Some(li)) =
            (log.block_number, log.transaction_hash, log.transaction_index, log.log_index)
        {
            columns.n_rows += 1;
            store!(schema, columns, block_number, bn as u32);
            store!(schema, columns, block_hash, log.block_hash.map(|bh| bh.to_vec()));
            store!(schema, columns, transaction_index, ti as u32);
            store!(schema, columns, log_index, li as u32);
            store!(schema, columns, transaction_hash, tx.to_vec());
            store!(schema, columns, contract_address, log.address().to_vec());
            // Indexed addresses sit left-padded in a 32-byte topic; keep the
            // low 20 bytes. The guard above proves both topics are present.
            store!(schema, columns, owner, log.topics()[1][12..].to_vec());
            store!(schema, columns, operator, log.topics()[2][12..].to_vec());
            store!(schema, columns, approved, decode_bool_word(&log.data().data));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::{Address, Bytes};

    /// real `ApprovalsForAll` schema, so `store!` actually evaluates each column
    fn schema() -> Table {
        Datatype::ApprovalsForAll
            .table_schema(
                &[U256Type::String],
                &ColumnEncoding::Hex,
                &None,
                &None,
                &None,
                None,
                None,
            )
            .expect("ApprovalsForAll has a valid default schema")
    }

    /// rpc `Log` carrying the block/tx/index fields `process_approvals_for_all` needs
    fn log_with(topics: Vec<B256>, data: Vec<u8>) -> Log {
        let inner = alloy::primitives::Log::new_unchecked(Address::ZERO, topics, Bytes::from(data));
        Log {
            inner,
            block_number: Some(1),
            transaction_hash: Some(B256::ZERO),
            transaction_index: Some(0),
            log_index: Some(0),
            ..Default::default()
        }
    }

    /// 32-byte ABI word holding `value` in its last byte
    fn word(value: u8) -> Vec<u8> {
        let mut data = vec![0u8; 32];
        data[31] = value;
        data
    }

    #[test]
    fn erc721_and_erc1155_share_one_topic0() {
        // The whole name of this dataset rests on this equality. If it ever
        // fails, one topic0 no longer covers both standards and the extractor
        // must filter on a list of two.
        assert_eq!(ERC721::ApprovalForAll::SIGNATURE_HASH, ERC1155::ApprovalForAll::SIGNATURE_HASH);
    }

    #[test]
    fn revocation_is_stored_as_false_not_dropped() {
        let logs = vec![
            log_with(vec![APPROVAL_FOR_ALL_TOPIC0, B256::ZERO, B256::ZERO], word(1)),
            log_with(vec![APPROVAL_FOR_ALL_TOPIC0, B256::ZERO, B256::ZERO], word(0)),
        ];
        let mut columns = ApprovalsForAll::default();
        process_approvals_for_all(logs, &mut columns, &schema()).unwrap();
        assert_eq!(columns.n_rows, 2, "a revocation is a row, not a missing row");
        assert_eq!(columns.approved, vec![true, false]);
    }

    #[test]
    fn a_dirty_bool_word_reads_as_true() {
        // Not written by solc, but the EVM would treat it as true, and reading
        // it as false would report a grant as a revocation.
        let log = log_with(vec![APPROVAL_FOR_ALL_TOPIC0, B256::ZERO, B256::ZERO], word(2));
        let mut columns = ApprovalsForAll::default();
        process_approvals_for_all(vec![log], &mut columns, &schema()).unwrap();
        assert_eq!(columns.approved, vec![true]);
    }

    #[test]
    fn owner_and_operator_come_from_topics_1_and_2() {
        let owner = B256::left_padding_from(&[0xaa; 20]);
        let operator = B256::left_padding_from(&[0xbb; 20]);
        let log = log_with(vec![APPROVAL_FOR_ALL_TOPIC0, owner, operator], word(1));
        let mut columns = ApprovalsForAll::default();
        process_approvals_for_all(vec![log], &mut columns, &schema()).unwrap();
        assert_eq!(columns.owner, vec![vec![0xaau8; 20]]);
        assert_eq!(columns.operator, vec![vec![0xbbu8; 20]]);
    }

    #[test]
    fn malformed_and_foreign_logs_are_skipped_not_panicking() {
        let missing_operator_topic = log_with(vec![APPROVAL_FOR_ALL_TOPIC0, B256::ZERO], word(1));
        let wrong_data_width =
            log_with(vec![APPROVAL_FOR_ALL_TOPIC0, B256::ZERO, B256::ZERO], vec![0u8; 8]);
        let foreign_topic0 =
            log_with(vec![B256::repeat_byte(0xfe), B256::ZERO, B256::ZERO], word(1));

        assert!(!is_approval_for_all(&missing_operator_topic));
        assert!(!is_approval_for_all(&wrong_data_width));
        assert!(!is_approval_for_all(&foreign_topic0));

        let mut columns = ApprovalsForAll::default();
        process_approvals_for_all(
            vec![missing_operator_topic, wrong_data_width, foreign_topic0],
            &mut columns,
            &schema(),
        )
        .unwrap();
        assert_eq!(columns.n_rows, 0);
    }
}
