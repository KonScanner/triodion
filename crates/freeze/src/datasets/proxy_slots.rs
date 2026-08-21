use crate::*;
use alloy::primitives::{b256, B256, U256};
use polars::prelude::*;

/// ERC-1967 implementation slot.
///
/// `keccak256("eip1967.proxy.implementation") - 1`. The `- 1` puts the slot
/// outside the image of `keccak256`, so no mapping or array can ever be laid
/// out on top of it.
const ERC1967_IMPLEMENTATION_SLOT: B256 =
    b256!("360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc");

/// ERC-1967 admin slot: `keccak256("eip1967.proxy.admin") - 1`.
const ERC1967_ADMIN_SLOT: B256 =
    b256!("b53127684a568b3173ae13b9f8a6016e243e63b6e8ee1178d6a717850b5d6103");

/// ERC-1967 beacon slot: `keccak256("eip1967.proxy.beacon") - 1`.
const ERC1967_BEACON_SLOT: B256 =
    b256!("a3f0ad74e5423aebfd80d3ef4346578335a9a72aeaee59ff6cb3582b35133d50");

// The fourth ERC-1967 slot, rollback
// (`0x4910fdfa16fed3260ed0e7147f7cc6da11a60208b5b9406d12a635614ffd9143`), is
// deliberately not read. It is written and cleared inside a single upgrade
// transaction, so any read at a block boundary sees it unset. A column of it
// would be all nulls and would invite the reading "no proxy here ever rolled
// back", which the data cannot support. Do not "fix" this omission.

/// columns for proxy_slots
///
/// One row per (block, address), holding the three ERC-1967 proxy slots read
/// with `eth_getStorageAt`.
///
/// A null means the slot is unset, i.e. the contract is not that kind of proxy
/// at that block. Every ordinary (non-proxy) contract answers null in all three
/// columns.
///
/// `implementation` set with `beacon` null is a standard ERC-1967 proxy: the
/// code it delegates to is in `implementation`. `beacon` set is a beacon proxy,
/// and its real implementation is held by the beacon contract, reachable only
/// through `IBeacon.implementation()` on that address — it is not in this row
/// and must not be inferred from it. The two columns are separate precisely so
/// no single column has to mean both things.
#[triodion_macros::to_df(Datatype::ProxySlots)]
#[derive(Default)]
pub struct ProxySlots {
    n_rows: u64,
    block_number: Vec<u32>,
    address: Vec<Vec<u8>>,
    implementation: Vec<Option<Vec<u8>>>,
    admin: Vec<Option<Vec<u8>>>,
    beacon: Vec<Option<Vec<u8>>>,
    chain_id: Vec<u64>,
}

impl Dataset for ProxySlots {
    fn default_columns() -> Option<Vec<&'static str>> {
        Some(vec!["block_number", "address", "implementation", "admin", "beacon", "chain_id"])
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

    fn aliases() -> Vec<&'static str> {
        vec!["erc1967_slots"]
    }

    fn arg_aliases() -> Option<std::collections::HashMap<Dim, Dim>> {
        Some([(Dim::Contract, Dim::Address)].into_iter().collect())
    }
}

impl CollectByBlock for ProxySlots {
    type Response = (u32, Vec<u8>, Option<Vec<u8>>, Option<Vec<u8>>, Option<Vec<u8>>);

    async fn extract(request: Params, source: Arc<Source>, query: Arc<Query>) -> R<Self::Response> {
        let address = request.ethers_address()?;
        let block_number = request.block_number()? as u32;

        // `from_be_bytes`, not `from_be_slice`: the latter panics when the slice
        // is not at most 32 bytes. All three are 32-byte constants, so neither
        // form can fail today — the total one is used so nothing is left to
        // inherit if one of these ever becomes a runtime value.
        let slots = [
            U256::from_be_bytes(ERC1967_IMPLEMENTATION_SLOT.0),
            U256::from_be_bytes(ERC1967_ADMIN_SLOT.0),
            U256::from_be_bytes(ERC1967_BEACON_SLOT.0),
        ];

        // Unlike an `eth_call` read of a getter, a storage read has no
        // contract-level failure to fold into a null: an address with no code
        // answers a zero word exactly as an unset slot does. So every error
        // here is a node error and must propagate, or the chunk would be
        // written as nulls. Neither path below ever turns a *batching* failure
        // into an error, for exactly that reason.
        let words = if query.batch_state_reads {
            // Three slots of one contract, which is one state-override call
            // rather than three `eth_getStorageAt`s. This dataset cannot use the
            // `StateOverrideBatchable` runner — that batches one word per *row*,
            // and a row here is three — so it reads through
            // `read_storage_slots`, which carries the same demotion policy at
            // the `Source` level and shares the same run-level verdict.
            //
            // Cross-address batching is not available to it either: `SLOAD`
            // reads only the storage of the contract currently executing, so the
            // extractor must be injected at each address in turn. The win is
            // 3 -> 1 requests per row, and there is no larger group to find.
            source.read_storage_slots(address, &slots, block_number.into()).await?
        } else {
            // `--no-batch-state-reads`. This is not the demotion path — that one
            // lives inside `read_storage_slots` and is reached by failure, never
            // by choice. It is the plain read the operator asked for, and the
            // same three requests this dataset made before batching existed.
            //
            // Every other state-read dataset routes on this flag in
            // `collect_by_block`; this one cannot, because the flag has to be
            // read where the three slots are, so it routes here instead. A
            // dataset that ignored the flag would make the switch a lie.
            //
            // All three requests go out at once, so turning batching off costs
            // requests, not wall-clock — the same property `per_row` gives the
            // runner.
            let reads =
                slots.iter().map(|slot| source.get_storage_at(address, *slot, block_number.into()));
            futures::future::try_join_all(reads)
                .await?
                .into_iter()
                // The same 32 big-endian bytes the batched path returns, so
                // both routes write byte-identical cells.
                .map(|word| B256::from(word.to_be_bytes::<32>()))
                .collect()
        };

        // One word per requested slot, in order, on both paths. A different
        // count is a bug in this crate, not a property of the chain, so it must
        // not become three nulls — that would read as "not a proxy".
        let [implementation, admin, beacon]: [B256; 3] = words.try_into().map_err(|_| {
            err("state read returned the wrong number of words for the ERC-1967 slots")
        })?;

        Ok((
            block_number,
            request.address()?,
            word_as_address(implementation),
            word_as_address(admin),
            word_as_address(beacon),
        ))
    }

    fn transform(response: Self::Response, columns: &mut Self, query: &Arc<Query>) -> R<()> {
        let schema = query.schemas.get_schema(&Datatype::ProxySlots)?;
        let (block, address, implementation, admin, beacon) = response;
        columns.n_rows += 1;
        store!(schema, columns, block_number, block);
        store!(schema, columns, address, address);
        store!(schema, columns, implementation, implementation);
        store!(schema, columns, admin, admin);
        store!(schema, columns, beacon, beacon);
        Ok(())
    }
}

// Storage is a property of a block, not of a transaction. `-t` would have to
// mean "the slots as of the block that contains this transaction", which is
// the same row for every transaction in that block and is already reachable by
// asking for the block.
impl CollectByTransaction for ProxySlots {
    type Response = ();
}

/// The address held in an ERC-1967 slot word, or `None` when the slot is unset.
fn word_as_address(word: B256) -> Option<Vec<u8>> {
    // A proxy itself reads these slots as `address(uint160(uint256(word)))`,
    // which keeps the low 20 bytes and drops the rest. Mirror that truncation
    // instead of rejecting a word whose high 12 bytes are dirty.
    let address = &word.0[12..];

    // Zero means the slot was never written, so this contract is not that kind
    // of proxy. Storing 0x00..00 would instead assert that it delegates to the
    // zero address — a live claim about a live proxy — and every non-proxy
    // contract in the file would then join against the zero-address row.
    (!address.iter().all(|byte| *byte == 0)).then(|| address.to_vec())
}
