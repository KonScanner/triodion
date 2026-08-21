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

/// Column values for `event_name`, spelled the way the ABI spells them. Every
/// dataset with an `event_name` column uses the ABI casing, so one column name
/// keeps one convention across the crate.
const DEPOSIT: &str = "Deposit";
const WITHDRAW: &str = "Withdraw";

/// columns for ERC-4626 tokenised vault `Deposit` and `Withdraw` events
///
/// Both events are kept in one table because they are the two halves of the
/// same movement: assets in against shares out, and shares in against assets
/// out. `event_name` says which half a row is.
///
/// ERC-4626's `Deposit(address,address,uint256,uint256)` is **not** the
/// WETH-style `Deposit(address,uint256)` that [`Erc20WrapperEvents`] collects.
/// The two signatures hash to different topic0 values, so the filters here and
/// there select disjoint sets of logs and a vault deposit can never turn up as
/// a wrapper deposit. Do not dedupe across the two datasets on that assumption.
///
/// There is deliberately no share-price column. The price is `assets / shares`,
/// a division whose scaling and rounding belong to whoever asks the question,
/// and `shares == 0` would make it undefined — a value this table would then
/// have to invent.
///
/// Matching happens at the event-signature level with no per-contract
/// configuration, so any contract emitting these exact signatures is indexed
/// whether or not it is a conforming vault.
#[triodion_macros::to_df(Datatype::Erc4626VaultEvents)]
#[derive(Default)]
pub struct Erc4626VaultEvents {
    n_rows: u64,
    block_number: Vec<u32>,
    block_hash: Vec<Option<Vec<u8>>>,
    transaction_index: Vec<u32>,
    log_index: Vec<u32>,
    transaction_hash: Vec<Vec<u8>>,
    /// the vault contract that emitted the event
    vault: Vec<Vec<u8>>,
    /// `"Deposit"` or `"Withdraw"`, spelled the way the ABI spells them so a row
    /// joins back to the event definition without a translation table.
    /// `"Withdraw"` is a different event from the `"withdrawal"` of
    /// [`Erc20WrapperEvents`].
    event_name: Vec<String>,
    /// the caller that moved the assets or shares
    sender: Vec<Vec<u8>>,
    /// the account whose shares were minted (`Deposit`) or burned (`Withdraw`)
    owner: Vec<Vec<u8>>,
    /// who received the assets. `Withdraw` names a receiver; `Deposit` has no
    /// receiver argument at all, so the column is null there.
    receiver: Vec<Option<Vec<u8>>>,
    /// underlying assets moved, in the vault's asset units
    assets: Vec<U256>,
    /// vault shares moved, in the vault's share units
    shares: Vec<U256>,
    chain_id: Vec<u64>,
}

impl Dataset for Erc4626VaultEvents {
    fn aliases() -> Vec<&'static str> {
        vec!["erc4626_events"]
    }

    fn default_columns() -> Option<Vec<&'static str>> {
        Some(vec![
            "block_number",
            // "block_hash",
            "transaction_index",
            "log_index",
            "transaction_hash",
            "vault",
            "event_name",
            "sender",
            "owner",
            "receiver",
            "assets",
            "shares",
            "chain_id",
        ])
    }

    fn optional_parameters() -> Vec<Dim> {
        // Topic0 is fixed to the two vault signatures. Topic1 is `sender` in
        // both events, so filtering on it means one thing. Topic2 and Topic3
        // are deliberately not offered: topic2 is `owner` on a Deposit but
        // `receiver` on a Withdraw, so one filter value would silently select
        // two different roles.
        vec![Dim::Address, Dim::Topic1]
    }

    fn use_block_ranges() -> bool {
        true
    }

    fn default_inner_request_size() -> u64 {
        // Same 50-block default as the other log datasets. Vault events are far
        // rarer per block than ERC-20 transfers, so this stays well inside any
        // node's response limits while keeping request counts predictable
        // across log-shaped datasets.
        50
    }

    fn arg_aliases() -> Option<std::collections::HashMap<Dim, Dim>> {
        Some([(Dim::Contract, Dim::Address)].into_iter().collect())
    }
}

impl CollectByBlock for Erc4626VaultEvents {
    type Response = Vec<Log>;

    async fn extract(request: Params, source: Arc<Source>, _: Arc<Query>) -> R<Self::Response> {
        let mut topics: [Topic; 4] = Default::default();
        // Union filter — eth_getLogs treats a list at one topic position as OR.
        topics[0] =
            vec![ERC4626::Deposit::SIGNATURE_HASH, ERC4626::Withdraw::SIGNATURE_HASH].into();
        if let Some(sender) = &request.topic1 {
            topics[1] = fixed_from_slice::<B256>(sender, "topic1")?.into();
        }
        let filter = Filter { topics, ..request.ethers_log_filter()? };
        let logs = source.get_logs(&filter).await?;
        Ok(logs.into_iter().filter(is_erc4626_vault_event).collect())
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::Erc4626VaultEvents)?;
        process_vault_events(response, columns, schema)
    }
}

impl CollectByTransaction for Erc4626VaultEvents {
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
                is_erc4626_vault_event(log) &&
                    log_address_matches(log, &request.address) &&
                    topic_matches(log, 1, &request.topic1)
            })
            .collect())
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::Erc4626VaultEvents)?;
        process_vault_events(response, columns, schema)
    }
}

/// which of the two ERC-4626 events a log carries
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum VaultEvent {
    Deposit,
    Withdraw,
}

/// The event a log carries, or `None` when it is neither. The topic count is
/// checked against the event's own declaration, not against a shared shape:
/// `Deposit` indexes two arguments and `Withdraw` indexes three, so a single
/// count would accept a malformed log of the other kind.
fn vault_event(log: &Log) -> Option<VaultEvent> {
    // Both events carry exactly two non-indexed uint256 words: assets, shares.
    if log.data().data.len() != 64 {
        return None
    }
    let topics = log.topics();
    match topics.first() {
        Some(t) if *t == ERC4626::Deposit::SIGNATURE_HASH && topics.len() == 3 => {
            Some(VaultEvent::Deposit)
        }
        Some(t) if *t == ERC4626::Withdraw::SIGNATURE_HASH && topics.len() == 4 => {
            Some(VaultEvent::Withdraw)
        }
        _ => None,
    }
}

/// True iff `log` is a well-formed ERC-4626 `Deposit` or `Withdraw`. Shared with
/// the per-transaction path, which receives every log of a transaction and must
/// pick these out itself.
pub(crate) fn is_erc4626_vault_event(log: &Log) -> bool {
    vault_event(log).is_some()
}

/// process logs into columns
fn process_vault_events(logs: Vec<Log>, columns: &mut Erc4626VaultEvents, schema: &Table) -> R<()> {
    for log in logs.iter() {
        // Re-checked rather than assumed: the topic indexing below would panic
        // on a log of the wrong shape, and a future caller may reach this loop
        // with logs that no filter narrowed first.
        let Some(event) = vault_event(log) else { continue };
        let (Some(bn), Some(tx), Some(ti), Some(li)) =
            (log.block_number, log.transaction_hash, log.transaction_index, log.log_index)
        else {
            continue
        };

        let topics = log.topics();
        // Withdraw(sender, receiver, owner) puts owner last; Deposit(sender,
        // owner) has no receiver argument at all. Copying owner into receiver
        // on a deposit row would invent a role the event never named, and any
        // "who was paid out" query would then count deposits as payouts.
        let (owner, receiver) = match event {
            VaultEvent::Deposit => (topics[2][12..].to_vec(), None),
            VaultEvent::Withdraw => (topics[3][12..].to_vec(), Some(topics[2][12..].to_vec())),
        };
        let event_name = match event {
            VaultEvent::Deposit => DEPOSIT,
            VaultEvent::Withdraw => WITHDRAW,
        };
        let data = &log.data().data;

        columns.n_rows += 1;
        store!(schema, columns, block_number, bn as u32);
        store!(schema, columns, block_hash, log.block_hash.map(|hash| hash.to_vec()));
        store!(schema, columns, transaction_index, ti as u32);
        store!(schema, columns, log_index, li as u32);
        store!(schema, columns, transaction_hash, tx.to_vec());
        store!(schema, columns, vault, log.address().to_vec());
        store!(schema, columns, event_name, event_name.to_string());
        // Indexed addresses arrive left-padded to 32 bytes; keep the low 20.
        store!(schema, columns, sender, topics[1][12..].to_vec());
        store!(schema, columns, owner, owner);
        store!(schema, columns, receiver, receiver);
        // `vault_event` proved the data is exactly 64 bytes, so neither slice
        // can be out of range. assets first, shares second, as declared.
        store!(schema, columns, assets, U256::from_be_slice(&data[..32]));
        store!(schema, columns, shares, U256::from_be_slice(&data[32..]));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::{Address, Bytes};

    /// real `Erc4626VaultEvents` schema, so `store!` actually evaluates each column
    fn schema() -> Table {
        Datatype::Erc4626VaultEvents
            .table_schema(
                &[U256Type::String],
                &ColumnEncoding::Hex,
                &None,
                &None,
                &None,
                None,
                None,
            )
            .expect("Erc4626VaultEvents has a valid default schema")
    }

    /// the two non-indexed words: assets then shares
    fn data(assets: u64, shares: u64) -> Vec<u8> {
        let mut out = U256::from(assets).to_be_bytes::<32>().to_vec();
        out.extend_from_slice(&U256::from(shares).to_be_bytes::<32>());
        out
    }

    /// rpc `Log` carrying the block/tx/index fields `process_vault_events` requires
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

    fn addr(byte: u8) -> B256 {
        B256::left_padding_from(&[byte; 20])
    }

    #[test]
    fn erc4626_deposit_does_not_share_topic0_with_the_weth_deposit() {
        // The whole reason both datasets can filter on "Deposit" without
        // colliding. If this ever fails, the two tables overlap.
        assert_ne!(ERC4626::Deposit::SIGNATURE_HASH, ERC20Wrapper::Deposit::SIGNATURE_HASH);
        assert_ne!(ERC4626::Withdraw::SIGNATURE_HASH, ERC20Wrapper::Withdrawal::SIGNATURE_HASH);
        assert_ne!(ERC4626::Deposit::SIGNATURE_HASH, ERC4626::Withdraw::SIGNATURE_HASH);
    }

    #[test]
    fn deposit_has_no_receiver() {
        let log =
            log_with(vec![ERC4626::Deposit::SIGNATURE_HASH, addr(0xaa), addr(0xbb)], data(7, 5));
        let mut columns = Erc4626VaultEvents::default();
        process_vault_events(vec![log], &mut columns, &schema()).unwrap();

        assert_eq!(columns.n_rows, 1);
        assert_eq!(columns.event_name, vec![DEPOSIT.to_string()]);
        assert_eq!(columns.sender, vec![vec![0xaau8; 20]]);
        assert_eq!(columns.owner, vec![vec![0xbbu8; 20]]);
        assert_eq!(columns.receiver, vec![None], "a deposit names no receiver");
        assert_eq!(columns.assets, vec![U256::from(7u64)]);
        assert_eq!(columns.shares, vec![U256::from(5u64)]);
    }

    #[test]
    fn withdraw_reads_owner_from_the_third_indexed_argument() {
        // topics: sig, sender, receiver, owner — owner last, not second.
        let log = log_with(
            vec![ERC4626::Withdraw::SIGNATURE_HASH, addr(0xaa), addr(0xbb), addr(0xcc)],
            data(9, 3),
        );
        let mut columns = Erc4626VaultEvents::default();
        process_vault_events(vec![log], &mut columns, &schema()).unwrap();

        assert_eq!(columns.event_name, vec![WITHDRAW.to_string()]);
        assert_eq!(columns.sender, vec![vec![0xaau8; 20]]);
        assert_eq!(columns.receiver, vec![Some(vec![0xbbu8; 20])]);
        assert_eq!(columns.owner, vec![vec![0xccu8; 20]]);
    }

    #[test]
    fn zero_shares_with_nonzero_assets_is_a_row() {
        // A donation-style deposit: assets go in, no shares come out. Legal,
        // and the row must survive so the vault's asset base stays reconcilable.
        let log = log_with(
            vec![ERC4626::Deposit::SIGNATURE_HASH, addr(0xaa), addr(0xbb)],
            data(1_000, 0),
        );
        let mut columns = Erc4626VaultEvents::default();
        process_vault_events(vec![log], &mut columns, &schema()).unwrap();

        assert_eq!(columns.n_rows, 1);
        assert_eq!(columns.shares, vec![U256::ZERO]);
        assert_eq!(columns.assets, vec![U256::from(1_000u64)]);
    }

    #[test]
    fn topic_count_is_checked_per_event_not_shared() {
        let d = ERC4626::Deposit::SIGNATURE_HASH;
        let w = ERC4626::Withdraw::SIGNATURE_HASH;
        // Deposit with a Withdraw's topic count, and the reverse.
        assert!(!is_erc4626_vault_event(&log_with(vec![d, addr(1), addr(2), addr(3)], data(1, 1))));
        assert!(!is_erc4626_vault_event(&log_with(vec![w, addr(1), addr(2)], data(1, 1))));
        assert!(is_erc4626_vault_event(&log_with(vec![d, addr(1), addr(2)], data(1, 1))));
        assert!(is_erc4626_vault_event(&log_with(vec![w, addr(1), addr(2), addr(3)], data(1, 1))));
    }

    #[test]
    fn wrong_data_width_and_foreign_topic0_are_skipped() {
        let logs = vec![
            log_with(vec![ERC4626::Deposit::SIGNATURE_HASH, addr(1), addr(2)], vec![0u8; 32]),
            log_with(vec![B256::repeat_byte(0xfe), addr(1), addr(2)], data(1, 1)),
            log_with(vec![], data(1, 1)),
        ];
        let mut columns = Erc4626VaultEvents::default();
        process_vault_events(logs, &mut columns, &schema()).unwrap();
        assert_eq!(columns.n_rows, 0);
    }

    #[test]
    fn skips_logs_missing_identity_fields() {
        let mut log =
            log_with(vec![ERC4626::Deposit::SIGNATURE_HASH, addr(1), addr(2)], data(1, 1));
        log.log_index = None;
        let mut columns = Erc4626VaultEvents::default();
        process_vault_events(vec![log], &mut columns, &schema()).unwrap();
        assert_eq!(columns.n_rows, 0);
    }
}
