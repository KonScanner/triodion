use crate::{CollectError, Datatype, Dim, MetaDatatype, Partition, Table};
use std::collections::{HashMap, HashSet};

/// Query
#[derive(Clone)]
pub struct Query {
    /// MetaDatatype
    pub datatypes: Vec<MetaDatatype>,
    /// Schemas for each subdatatype
    pub schemas: HashMap<Datatype, Table>,
    /// Time dimension
    pub time_dimension: TimeDimension,
    /// MetaChunks
    pub partitions: Vec<Partition>,
    /// Partitioning
    pub partitioned_by: Vec<Dim>,
    /// Exclude failed
    pub exclude_failed: bool,
    /// Javascript tracer
    pub js_tracer: Option<String>,
    /// Labels (these are non-functional)
    pub labels: QueryLabels,
    /// Batch `eth_call` invocations through Multicall3 (currently only `eth_calls`).
    ///
    /// When false, each call is dispatched as an individual `eth_call`. When
    /// true, calls sharing a block are aggregated through Multicall3 in chunks
    /// of [`multicall_batch_size`](Self::multicall_batch_size), with a halving
    /// fallback on RPC error and a per-call fallback at blocks earlier than the
    /// Multicall3 deploy block on the active chain.
    pub multicall: bool,
    /// Maximum number of inner calls per Multicall3 batch.
    ///
    /// Only meaningful when [`multicall`](Self::multicall) is true. Defaults to
    /// [`DEFAULT_MULTICALL_BATCH_SIZE`](crate::DEFAULT_MULTICALL_BATCH_SIZE)
    /// when constructed via the CLI / Python bindings.
    pub multicall_batch_size: u32,
    /// Read storage slots / balances in bulk through `eth_call` state overrides.
    ///
    /// When true, datasets that implement
    /// [`StateOverrideBatchable`](crate::types::state_override::StateOverrideBatchable)
    /// group their rows by `(block, contract)` and read each group with one
    /// `eth_call` carrying an extractor-bytecode `code` override, instead of one
    /// `eth_getStorageAt` / `eth_getBalance` per row. Any batch that fails —
    /// including on an endpoint that rejects or ignores overrides — degrades to
    /// the per-row path, so leaving this on cannot change results, only the
    /// number of requests it takes to get them.
    pub batch_state_reads: bool,
    /// Words per state-override extractor call.
    ///
    /// Only meaningful when [`batch_state_reads`](Self::batch_state_reads) is true.
    /// Defaults to
    /// [`DEFAULT_STATE_OVERRIDE_BATCH_SIZE`](crate::types::state_override::DEFAULT_STATE_OVERRIDE_BATCH_SIZE)
    /// when zero.
    pub state_override_batch_size: u32,
    /// Send many identical JSON-RPC calls per HTTP request.
    ///
    /// When true, datasets that implement
    /// [`RpcBatchable`](crate::types::rpc_batch::RpcBatchable) pack their rows
    /// into JSON-RPC batch envelopes — one HTTP request carrying N embedded
    /// calls — instead of one request per row. `blocks`, `nonces` and `codes`
    /// take this route.
    ///
    /// Deliberately separate from
    /// [`batch_state_reads`](Self::batch_state_reads). A state override needs a
    /// node that honours the third `eth_call` parameter, which not every
    /// endpoint does; a JSON-RPC batch needs nothing beyond JSON-RPC itself. An
    /// operator who distrusts an endpoint's override support has no reason to
    /// give up plain batching as well.
    ///
    /// A batch that fails falls back to the per-row path, so this changes how
    /// many requests the results cost, never the results.
    pub batch_rpc_calls: bool,
    /// When true, mark the entire Multicall3 batch as failed if any inner call reverts.
    ///
    /// When false (the default), inner reverts are returned as `None` in
    /// `output_data` — matching the existing per-call behaviour of
    /// `Source::call().await.ok()`.
    pub multicall_require_success: bool,
}

/// query labels (non-functional)
#[derive(Clone)]
pub struct QueryLabels {
    /// align
    pub align: bool,
    /// reorg buffer
    pub reorg_buffer: u64,
}

impl Query {
    /// total number of tasks needed to perform query
    pub fn n_tasks(&self) -> usize {
        self.datatypes.len() * self.partitions.len()
    }

    /// total number of outputs of query
    pub fn n_outputs(&self) -> usize {
        self.datatypes.iter().map(|x| x.datatypes().len()).sum::<usize>() * self.partitions.len()
    }

    /// check that query is valid
    pub fn is_valid(&self) -> Result<(), CollectError> {
        // check that required parameters are present
        let mut all_datatypes = std::collections::HashSet::new();
        for datatype in self.datatypes.iter() {
            all_datatypes.extend(datatype.datatypes())
        }
        let mut requirements: HashSet<Dim> = HashSet::new();
        for datatype in all_datatypes.iter() {
            for dim in datatype.required_parameters() {
                requirements.insert(dim);
            }
        }
        for partition in self.partitions.iter() {
            let partition_dims = partition.dims().into_iter().collect();
            if !requirements.is_subset(&partition_dims) {
                let missing: Vec<_> =
                    requirements.difference(&partition_dims).map(|x| x.to_string()).collect();
                return Err(CollectError::CollectError(format!(
                    "need to specify {}",
                    missing.join(", ")
                )))
            }
        }
        Ok(())
    }
}

/// Time dimension for queries
#[derive(Clone)]
pub enum TimeDimension {
    /// Blocks
    Blocks,
    /// Transactions
    Transactions,
}
