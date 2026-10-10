/// type specifications for triodion_core crate
/// consensus-layer (beacon chain) access
pub mod beacon;
/// cross-chain-family plumbing (Ethereum / OP stack / Arbitrum stack)
pub mod chains;
/// type specifications for chunk types
pub mod chunks;
/// conversion operations
pub mod conversions;
/// type specifications for collectable types
pub mod datatypes;
/// type specifications for data sources
pub mod sources;
/// wire-format block fixtures shared by the dataset tests
#[cfg(test)]
pub(crate) mod wire_fixtures;

/// column data specification
pub mod columns;
pub use columns::{ColumnData, Dataset, ToDataFrames};

/// partitions
pub mod partitions;
/// rpc_params
pub mod rpc_params;

pub use partitions::{Dim, Partition, PartitionLabels};
pub use rpc_params::{Params, address_dim_as_topic};

/// collection traits
pub mod collection;

/// execution environment
pub mod execution;

/// report generation
pub mod reports;
pub use reports::TRIODION_VERSION;

/// type specifications for dataframes
#[macro_use]
pub mod dataframes;

/// function and event signatures
#[allow(missing_docs)]
pub mod signatures;

/// Multicall3 helpers
pub mod multicall;

/// bulk state reads via `eth_call` state overrides
pub mod state_override;

/// many identical JSON-RPC calls per HTTP request
pub mod rpc_batch;

/// error specifications
pub mod errors;
/// type specifications for output data formats
pub mod files;
/// queries
pub mod queries;
/// type specifications for data schemas
pub mod schemas;
/// types related to summaries
pub mod summaries;

pub use beacon::{
    BeaconConfig, BeaconSource, BlobProvenance, BlobRecord, BlobSidecar, DEFAULT_BLOB_ARCHIVE,
};
pub use chains::{
    ChainFamily, RpcBlock, RpcReceipt, RpcTransaction, TriodionNetwork, TriodionProvider, TxExtras,
    arbitrum, is_reencodable, op, other_bool, other_bytes, other_decimal_f64, other_u64,
    other_u256,
};
pub use chunks::{
    AddressChunk, BlockChunk, CallDataChunk, Chunk, ChunkData, ChunkStats, SlotChunk, Subchunk,
    TopicChunk, TransactionChunk,
};
pub use conversions::{ToVecHex, ToVecU8, bytes_to_u32, decode_u256_word};
pub use dataframes::*;
pub use datatypes::*;
pub use files::{ColumnEncoding, FileFormat, FileOutput, SubDir};
pub use queries::{Query, QueryLabels, TimeDimension};
pub use rpc_batch::{DEFAULT_RPC_BATCH_ROWS, RpcBatchable, rpc_batch_collect_by_block};
pub use schemas::{ColumnType, SchemaFunctions, Schemas, Table, U256Type};
pub use sources::{
    Fetcher, RateLimiter, Source, SourceConfig, SourceLabels, new_rate_limiter, with_default_scheme,
};
pub use state_override::{
    DEFAULT_STATE_OVERRIDE_BATCH_SIZE, OverrideSupport, SCRATCH_ADDRESS, StateOverrideBatchable,
    StateReader, override_unavailable, state_override_collect_by_block,
};
// pub(crate) use summaries::FreezeSummaryAgg;
// pub use summaries::{FreezeChunkSummary, FreezeSummary};
pub use summaries::{FreezeSummary, print_all_datasets, print_dataset_info};

pub use errors::{
    CallOutcome, ChunkError, CollectError, FileError, FreezeError, ParseError, R, contract_read,
    err,
};

pub use collection::*;
pub use execution::{ExecutionEnv, ExecutionEnvBuilder};

pub use signatures::*;

pub use multicall::{
    DEFAULT_MULTICALL_BATCH_SIZE, MULTICALL3_ADDRESS, Multicall3, Multicall3Info,
    MulticallBatchable, decode_string_or_bytes32, default_collect_by_block, extract_by_eth_call,
    multicall_collect_by_block, multicall3_info,
};

/// decoders
pub mod decoders;
pub use decoders::*;
