use crate::{types::rpc_params::log_address_matches, *};
use alloy::{
    rpc::types::{Filter, Log, Topic},
    sol_types::SolEvent,
};
use polars::prelude::*;

/// Column values for `event_name`, spelled the way the ABI spells them so a row
/// joins back to the event definition without a translation table.
const UPGRADED: &str = "Upgraded";
const BEACON_UPGRADED: &str = "BeaconUpgraded";
const ADMIN_CHANGED: &str = "AdminChanged";

/// `AdminChanged` carries two non-indexed `address` arguments, so its data is
/// exactly two ABI words.
const ADMIN_CHANGED_DATA_LEN: usize = 64;

/// One ABI word, which is how a non-indexed `address` argument is encoded.
const ADDRESS_WORD_LEN: usize = 32;

/// columns for ERC-1967 proxy upgrade events
///
/// One row per emitted event, over all three ERC-1967 events:
/// `Upgraded(address indexed implementation)`,
/// `BeaconUpgraded(address indexed beacon)` and
/// `AdminChanged(address previousAdmin, address newAdmin)`.
///
/// The columns an event does not carry are null, not zero. The zero address is
/// a real value here and must stay distinguishable from an absence: a proxy
/// upgraded to `0x0` is bricked, and an admin changed to `0x0` has been
/// renounced. Three separate tables would each be near-empty and would have to
/// be merged again before "what changed on this proxy, and in what order" could
/// be asked, so this is one table.
///
/// This is the event-side view of a proxy. `proxy_slots` reads the ERC-1967
/// storage slots and answers "what is the implementation at block N"; this
/// dataset answers "when did it change, and in which transaction". Join
/// `proxy_upgrades.proxy_address` to `proxy_slots.address`. Neither replaces
/// the other:
///
/// - These events are a convention, not an EVM rule. A proxy can write the implementation slot and
///   emit nothing, and a beacon that swaps its own implementation moves every proxy behind it with
///   no event at any of them. No rows here is not evidence that nothing changed — only a slot read
///   is.
/// - A slot read cannot say how many times the slot changed between two blocks, or which
///   transaction changed it.
#[triodion_macros::to_df(Datatype::ProxyUpgrades)]
#[derive(Default)]
pub struct ProxyUpgrades {
    n_rows: u64,
    block_number: Vec<u32>,
    block_hash: Vec<Option<Vec<u8>>>,
    transaction_index: Vec<u32>,
    log_index: Vec<u32>,
    transaction_hash: Vec<Vec<u8>>,
    /// the contract that emitted the event: the proxy itself, never the
    /// implementation it points at
    proxy_address: Vec<Vec<u8>>,
    /// `Upgraded`, `BeaconUpgraded` or `AdminChanged`
    event_name: Vec<String>,
    /// `Upgraded` rows only
    implementation: Vec<Option<Vec<u8>>>,
    /// `BeaconUpgraded` rows only. The implementation behind the beacon is not
    /// in this log and must be read from the beacon contract.
    beacon: Vec<Option<Vec<u8>>>,
    /// `AdminChanged` rows only
    previous_admin: Vec<Option<Vec<u8>>>,
    /// `AdminChanged` rows only
    new_admin: Vec<Option<Vec<u8>>>,
    chain_id: Vec<u64>,
}

impl Dataset for ProxyUpgrades {
    fn aliases() -> Vec<&'static str> {
        vec!["erc1967_events"]
    }

    fn default_columns() -> Option<Vec<&'static str>> {
        Some(vec![
            "block_number",
            // "block_hash",
            "transaction_index",
            "log_index",
            "transaction_hash",
            "proxy_address",
            "event_name",
            "implementation",
            "beacon",
            "previous_admin",
            "new_admin",
            "chain_id",
        ])
    }

    fn optional_parameters() -> Vec<Dim> {
        // Topic0 is fixed to the three ERC-1967 signatures. Topic1 is
        // deliberately not offered: it is the implementation on `Upgraded`, the
        // beacon on `BeaconUpgraded`, and does not exist on `AdminChanged`, so
        // any `--topic1` value would silently delete every `AdminChanged` row
        // and mean two different things across the rows it kept.
        vec![Dim::Address]
    }

    fn use_block_ranges() -> bool {
        true
    }

    fn default_inner_request_size() -> u64 {
        // The same 50 as the other log datasets. A larger value would fetch
        // these rare events in fewer requests, but the dispatcher applies
        // `max(user, dataset_default)`, so a high default becomes a floor the
        // user cannot lower on a strict RPC endpoint.
        50
    }

    fn arg_aliases() -> Option<std::collections::HashMap<Dim, Dim>> {
        Some([(Dim::Contract, Dim::Address)].into_iter().collect())
    }
}

impl CollectByBlock for ProxyUpgrades {
    type Response = Vec<Log>;

    async fn extract(request: Params, source: Arc<Source>, _: Arc<Query>) -> R<Self::Response> {
        let mut topics: [Topic; 4] = Default::default();
        // A list at a topic position is OR in `eth_getLogs`, and alloy's `Topic`
        // is built from a `Vec<B256>` for exactly that. Assigning a single hash
        // instead would drop the other two events without any error.
        topics[0] = vec![
            ERC1967::Upgraded::SIGNATURE_HASH,
            ERC1967::BeaconUpgraded::SIGNATURE_HASH,
            ERC1967::AdminChanged::SIGNATURE_HASH,
        ]
        .into();
        let filter = Filter { topics, ..request.ethers_log_filter()? };
        let logs = source.get_logs(&filter).await?;
        // The node matched topic0 only; this also checks the shape each event
        // must have before its arguments can be read.
        Ok(logs.into_iter().filter(is_proxy_upgrade).collect())
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::ProxyUpgrades)?;
        process_proxy_upgrades(response, columns, schema)
    }
}

impl CollectByTransaction for ProxyUpgrades {
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
            .filter(|log| is_proxy_upgrade(log) && log_address_matches(log, &request.address))
            .collect())
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::ProxyUpgrades)?;
        process_proxy_upgrades(response, columns, schema)
    }
}

/// One decoded ERC-1967 event. The variants carry only the arguments their event
/// actually has, so the null columns of a row are decided here rather than by a
/// default filled in later.
enum ProxyUpgradeEvent {
    Upgraded { implementation: Vec<u8> },
    BeaconUpgraded { beacon: Vec<u8> },
    AdminChanged { previous_admin: Vec<u8>, new_admin: Vec<u8> },
}

/// True iff `log` is one of the three ERC-1967 events and has the shape that
/// event's arguments live in. Written to stay reusable by a coalesced [`crate::LogEvents`] fan-out,
/// which this datatype is NOT a member of today (see `MultiDatatype::LogEvents`).
pub(crate) fn is_proxy_upgrade(log: &Log) -> bool {
    let Some(topic0) = log.topics().first() else { return false };
    if *topic0 == ERC1967::Upgraded::SIGNATURE_HASH ||
        *topic0 == ERC1967::BeaconUpgraded::SIGNATURE_HASH
    {
        // Two encodings of the same event, both live on mainnet.
        //
        // ERC-1967 and OpenZeppelin >= 3.x declare the address argument
        // `indexed`, so it is topics[1] and the data is empty. The older
        // zos-lib `AdminUpgradeabilityProxy` declares `Upgraded(address
        // implementation)` with no `indexed`, so the log has one topic and the
        // address sits in a single 32-byte data word.
        //
        // `indexed` is not part of the signature preimage, so both hash to the
        // same topic0 and the node returns both for the filter in `extract`.
        // Accepting only the topic form silently dropped every upgrade of a
        // zos-lib proxy — USDC's is one — and reported the proxy as never
        // upgraded.
        //
        // Extra data alongside a present topic1 is tolerated rather than
        // rejected: the argument is still unambiguously in the topic, and
        // dropping the row would lose a real upgrade over padding nobody
        // reads. The width must be exact in the data form, because there the
        // padding is the value.
        log.topics().len() >= 2 || log.data().data.len() == ADDRESS_WORD_LEN
    } else if *topic0 == ERC1967::AdminChanged::SIGNATURE_HASH {
        // The opposite case, and the one that is easy to get backwards.
        // `AdminChanged` indexes neither argument, so the log has one topic and
        // both admins sit in the data. The width must be exact — the ABI decode
        // below reads two words, and a different width means some other event
        // collided with this signature.
        log.data().data.len() == ADMIN_CHANGED_DATA_LEN
    } else {
        false
    }
}

/// The sole `address` argument of `Upgraded` or `BeaconUpgraded`, from whichever
/// of the two encodings this log uses.
///
/// A present topic1 wins: the argument is `indexed`, so the topic is the value
/// and any data is padding. Otherwise the argument is non-indexed and the data
/// holds it as one ABI word.
///
/// The high 12 bytes of that word must be zero. An `address` is ABI-encoded
/// left-padded with zeros, so a non-zero prefix means the 32 bytes are not an
/// address — some other event collided with this signature — and inventing an
/// address from its low 20 bytes would be worse than skipping the row.
fn address_argument(log: &Log) -> Option<Vec<u8>> {
    if let Some(topic) = log.topics().get(1) {
        return Some(topic[12..].to_vec())
    }
    let word = log.data().data.as_ref();
    if word.len() != ADDRESS_WORD_LEN || word[..12].iter().any(|byte| *byte != 0) {
        return None
    }
    Some(word[12..].to_vec())
}

/// Decode one log, or `None` if it is not a well-formed ERC-1967 event.
fn decode_proxy_upgrade(log: &Log) -> Option<ProxyUpgradeEvent> {
    let topic0 = *log.topics().first()?;
    if topic0 == ERC1967::Upgraded::SIGNATURE_HASH {
        let implementation = address_argument(log)?;
        Some(ProxyUpgradeEvent::Upgraded { implementation })
    } else if topic0 == ERC1967::BeaconUpgraded::SIGNATURE_HASH {
        let beacon = address_argument(log)?;
        Some(ProxyUpgradeEvent::BeaconUpgraded { beacon })
    } else if topic0 == ERC1967::AdminChanged::SIGNATURE_HASH {
        if log.data().data.len() != ADMIN_CHANGED_DATA_LEN {
            return None
        }
        // Read from the data, not the topics. `decode_log_data` re-checks the
        // signature and topic count, and a failure here skips the row.
        let decoded = ERC1967::AdminChanged::decode_log_data(log.data()).ok()?;
        Some(ProxyUpgradeEvent::AdminChanged {
            previous_admin: decoded.previousAdmin.to_vec(),
            new_admin: decoded.newAdmin.to_vec(),
        })
    } else {
        None
    }
}

/// process logs into columns
fn process_proxy_upgrades(logs: Vec<Log>, columns: &mut ProxyUpgrades, schema: &Table) -> R<()> {
    for log in logs.iter() {
        // Decode before anything is stored. A caller that reaches here with
        // mixed logs — a future coalesced [`crate::LogEvents`] fan-out, say —
        // skips them here instead of writing a row of nulls under a real
        // block number, which would read as "an upgrade with no argument".
        let Some(event) = decode_proxy_upgrade(log) else { continue };

        // Each arm names every column the event does not have. An `Upgraded`
        // row has no admin at all, which is not the same as an admin of zero.
        let (event_name, implementation, beacon, previous_admin, new_admin) = match event {
            ProxyUpgradeEvent::Upgraded { implementation } => {
                (UPGRADED, Some(implementation), None, None, None)
            }
            ProxyUpgradeEvent::BeaconUpgraded { beacon } => {
                (BEACON_UPGRADED, None, Some(beacon), None, None)
            }
            ProxyUpgradeEvent::AdminChanged { previous_admin, new_admin } => {
                (ADMIN_CHANGED, None, None, Some(previous_admin), Some(new_admin))
            }
        };

        if let (Some(bn), Some(tx), Some(ti), Some(li)) =
            (log.block_number, log.transaction_hash, log.transaction_index, log.log_index)
        {
            columns.n_rows += 1;
            store!(schema, columns, block_number, bn as u32);
            store!(schema, columns, block_hash, log.block_hash.map(|bh| bh.to_vec()));
            store!(schema, columns, transaction_index, ti as u32);
            store!(schema, columns, log_index, li as u32);
            store!(schema, columns, transaction_hash, tx.to_vec());
            store!(schema, columns, proxy_address, log.address().to_vec());
            store!(schema, columns, event_name, event_name.to_string());
            store!(schema, columns, implementation, implementation);
            store!(schema, columns, beacon, beacon);
            store!(schema, columns, previous_admin, previous_admin);
            store!(schema, columns, new_admin, new_admin);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::{Address, Bytes, B256};

    /// real `ProxyUpgrades` schema, so `store!` actually evaluates each column
    fn schema() -> Table {
        Datatype::ProxyUpgrades
            .table_schema(
                &[U256Type::String],
                &ColumnEncoding::Hex,
                &None,
                &None,
                &None,
                None,
                None,
            )
            .expect("ProxyUpgrades has a valid default schema")
    }

    /// rpc `Log` carrying the block/tx/index fields `process_proxy_upgrades` needs
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

    /// the two ABI words an `AdminChanged` log carries in its data
    fn admin_changed_data(previous: [u8; 20], new: [u8; 20]) -> Vec<u8> {
        let mut data = Vec::with_capacity(ADMIN_CHANGED_DATA_LEN);
        data.extend_from_slice(B256::left_padding_from(&previous).as_slice());
        data.extend_from_slice(B256::left_padding_from(&new).as_slice());
        data
    }

    #[test]
    fn the_three_signatures_are_distinct() {
        let upgraded = ERC1967::Upgraded::SIGNATURE_HASH;
        let beacon = ERC1967::BeaconUpgraded::SIGNATURE_HASH;
        let admin = ERC1967::AdminChanged::SIGNATURE_HASH;
        assert_ne!(upgraded, beacon);
        assert_ne!(upgraded, admin);
        assert_ne!(beacon, admin);
    }

    #[test]
    fn admin_changed_is_read_from_data_not_topics() {
        // The headline trap: neither argument is indexed. A topics-based read
        // would find nothing and index past the end of a one-topic log.
        let log = log_with(
            vec![ERC1967::AdminChanged::SIGNATURE_HASH],
            admin_changed_data([0x11; 20], [0x22; 20]),
        );
        assert_eq!(log.topics().len(), 1, "AdminChanged has no indexed argument");

        let mut columns = ProxyUpgrades::default();
        process_proxy_upgrades(vec![log], &mut columns, &schema()).unwrap();

        assert_eq!(columns.n_rows, 1);
        assert_eq!(columns.event_name, vec![ADMIN_CHANGED.to_string()]);
        assert_eq!(columns.previous_admin, vec![Some(vec![0x11u8; 20])]);
        assert_eq!(columns.new_admin, vec![Some(vec![0x22u8; 20])]);
        // Null, not the zero address: this event carries no implementation.
        assert_eq!(columns.implementation, vec![None]);
        assert_eq!(columns.beacon, vec![None]);
    }

    #[test]
    fn upgraded_is_read_from_topic1() {
        let implementation = B256::left_padding_from(&[0xab; 20]);
        let log = log_with(vec![ERC1967::Upgraded::SIGNATURE_HASH, implementation], vec![]);

        let mut columns = ProxyUpgrades::default();
        process_proxy_upgrades(vec![log], &mut columns, &schema()).unwrap();

        assert_eq!(columns.n_rows, 1);
        assert_eq!(columns.event_name, vec![UPGRADED.to_string()]);
        assert_eq!(columns.implementation, vec![Some(vec![0xabu8; 20])]);
        assert_eq!(columns.beacon, vec![None]);
        assert_eq!(columns.previous_admin, vec![None]);
        assert_eq!(columns.new_admin, vec![None]);
    }

    #[test]
    fn beacon_upgraded_fills_beacon_and_leaves_implementation_null() {
        // A beacon proxy's implementation lives on the beacon, not in this log.
        // Filling it with the beacon address would make the two indistinguishable.
        let beacon = B256::left_padding_from(&[0xcd; 20]);
        let log = log_with(vec![ERC1967::BeaconUpgraded::SIGNATURE_HASH, beacon], vec![]);

        let mut columns = ProxyUpgrades::default();
        process_proxy_upgrades(vec![log], &mut columns, &schema()).unwrap();

        assert_eq!(columns.n_rows, 1);
        assert_eq!(columns.event_name, vec![BEACON_UPGRADED.to_string()]);
        assert_eq!(columns.beacon, vec![Some(vec![0xcdu8; 20])]);
        assert_eq!(columns.implementation, vec![None]);
    }

    #[test]
    fn upgraded_with_no_topic1_and_no_data_is_skipped_not_panicking() {
        // One topic and an empty body names no address in either encoding, so
        // there is nothing to report and nothing to panic on.
        let malformed = log_with(vec![ERC1967::Upgraded::SIGNATURE_HASH], vec![]);
        assert!(!is_proxy_upgrade(&malformed));

        let mut columns = ProxyUpgrades::default();
        process_proxy_upgrades(vec![malformed], &mut columns, &schema()).unwrap();
        assert_eq!(columns.n_rows, 0);
    }

    #[test]
    fn upgraded_is_read_from_the_data_when_the_argument_is_not_indexed() {
        // The zos-lib `AdminUpgradeabilityProxy` form: `Upgraded(address
        // implementation)` with no `indexed`. `indexed` is not in the signature
        // preimage, so this log carries the same topic0 as the ERC-1967 form
        // and the node returns it for the same filter. Reading only topics[1]
        // dropped every one of these, USDC's proxy included.
        let mut word = vec![0u8; 32];
        word[12..].copy_from_slice(&[0xab; 20]);
        let log = log_with(vec![ERC1967::Upgraded::SIGNATURE_HASH], word);
        assert!(is_proxy_upgrade(&log));

        let mut columns = ProxyUpgrades::default();
        process_proxy_upgrades(vec![log], &mut columns, &schema()).unwrap();

        assert_eq!(columns.n_rows, 1);
        assert_eq!(columns.event_name, vec![UPGRADED.to_string()]);
        assert_eq!(columns.implementation, vec![Some(vec![0xabu8; 20])]);
        assert_eq!(columns.beacon, vec![None]);
    }

    #[test]
    fn a_data_word_that_is_not_an_address_is_skipped() {
        // An `address` is ABI-encoded left-padded with zeros. A non-zero high
        // prefix means these 32 bytes are something else that collided with
        // this signature, and its low 20 bytes are not an implementation.
        let log = log_with(vec![ERC1967::Upgraded::SIGNATURE_HASH], vec![0xff; 32]);

        let mut columns = ProxyUpgrades::default();
        process_proxy_upgrades(vec![log], &mut columns, &schema()).unwrap();
        assert_eq!(columns.n_rows, 0);
    }

    #[test]
    fn admin_changed_with_wrong_data_width_is_skipped() {
        let short = log_with(vec![ERC1967::AdminChanged::SIGNATURE_HASH], vec![0u8; 32]);
        assert!(!is_proxy_upgrade(&short));

        let mut columns = ProxyUpgrades::default();
        process_proxy_upgrades(vec![short], &mut columns, &schema()).unwrap();
        assert_eq!(columns.n_rows, 0);
    }

    #[test]
    fn indexed_events_tolerate_trailing_data() {
        // The argument is in the topic, so unexpected data does not make the
        // row ambiguous and must not delete a real upgrade.
        let implementation = B256::left_padding_from(&[0x07; 20]);
        let log = log_with(vec![ERC1967::Upgraded::SIGNATURE_HASH, implementation], vec![0xff; 32]);
        assert!(is_proxy_upgrade(&log));

        let mut columns = ProxyUpgrades::default();
        process_proxy_upgrades(vec![log], &mut columns, &schema()).unwrap();
        assert_eq!(columns.implementation, vec![Some(vec![0x07u8; 20])]);
    }

    #[test]
    fn foreign_topic0_is_skipped() {
        let logs = vec![
            log_with(vec![B256::repeat_byte(0xfe), B256::ZERO], vec![]),
            log_with(vec![ERC1967::Upgraded::SIGNATURE_HASH, B256::ZERO], vec![]),
        ];
        let mut columns = ProxyUpgrades::default();
        process_proxy_upgrades(logs, &mut columns, &schema()).unwrap();
        assert_eq!(columns.n_rows, 1);
        assert_eq!(columns.event_name, vec![UPGRADED.to_string()]);
    }

    #[test]
    fn skips_logs_missing_identity_fields() {
        let mut log = log_with(vec![ERC1967::Upgraded::SIGNATURE_HASH, B256::ZERO], vec![]);
        log.log_index = None;
        let mut columns = ProxyUpgrades::default();
        process_proxy_upgrades(vec![log], &mut columns, &schema()).unwrap();
        assert_eq!(columns.n_rows, 0);
    }

    #[test]
    fn a_zero_implementation_is_a_value_not_an_absence() {
        // A proxy upgraded to 0x0 is bricked. That must not read as "unknown".
        let log = log_with(vec![ERC1967::Upgraded::SIGNATURE_HASH, B256::ZERO], vec![]);
        let mut columns = ProxyUpgrades::default();
        process_proxy_upgrades(vec![log], &mut columns, &schema()).unwrap();
        assert_eq!(columns.implementation, vec![Some(vec![0u8; 20])]);
    }
}
