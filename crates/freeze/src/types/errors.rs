use alloy::transports::{RpcError, TransportErrorKind};
/// error specifications
use polars::prelude::*;
use thiserror::Error;

/// standard CollectError Result
pub type R<T> = ::core::result::Result<T, CollectError>;

/// return basic CollectError from str slice
pub fn err(message: &str) -> CollectError {
    CollectError::CollectError(message.to_string())
}

/// Error related to running freeze function
#[derive(Error, Debug)]
pub enum FreezeError {
    /// Error related to processing file path
    #[error(transparent)]
    FilePathError(#[from] FileError),

    /// Error related to joining a tokio task
    #[error("Task failed: {0}")]
    TaskFailed(#[source] tokio::task::JoinError),

    /// Error related to collecting data
    #[error(transparent)]
    CollectError(#[from] CollectError),

    /// Error related to progress bar
    #[error("Progress bar error")]
    ProgressBarError(#[from] indicatif::style::TemplateError),

    /// Parse error
    #[error(transparent)]
    ParseError(#[from] ParseError),

    /// Error from serializing report
    #[error("JSON error")]
    ReportSerializeError(#[from] serde_json::Error),

    /// Error from serializing report
    #[error("File creation error")]
    ReportFileCreationError(#[from] std::io::Error),

    /// General Error
    #[error("{0}")]
    GeneralError(String),
}

/// Error related to data collection
#[derive(Error, Debug)]
pub enum CollectError {
    /// General Collection error
    #[error("Collect failed: {0}")]
    CollectError(String),

    /// Parse error
    #[error(transparent)]
    ParseError(#[from] ParseError),

    /// Error related to provider operations
    #[error("Failed to get block: {0}")]
    ProviderError(#[source] RpcError<TransportErrorKind>),

    /// Error related to tokio task
    #[error("Task failed: {0}")]
    TaskFailed(#[source] tokio::task::JoinError),

    /// Error related to polars operations
    #[error("Failed to convert to DataFrame: {0}")]
    PolarsError(#[from] PolarsError),

    /// Error related to log topic filtering
    #[error("Invalid number of topics")]
    InvalidNumberOfTopics,

    /// Error related to bad schema
    #[error("Bad schema specified")]
    BadSchemaError,

    /// Error related to too many requests
    #[error("try using a rate limit with --requests-per-second or limiting max concurrency with --max-concurrent-requests")]
    TooManyRequestsError,

    /// Generic RPC Error
    #[error("RPC call error")]
    RPCError(String),
}

/// Error related to parsing
#[derive(Error, Debug)]
pub enum ParseError {
    /// Error related to parsing
    #[error("Parsing error: {0}")]
    ParseError(String),

    /// Error related to provider operations
    #[error("Failed to get block: {0}")]
    ProviderError(#[source] RpcError<TransportErrorKind>),

    /// Parse int error
    #[error("Parsing error")]
    ParseIntError(#[from] std::num::ParseIntError),

    /// MESC error
    #[error("MESC error: {:?}", .0)]
    MescError(mesc::MescError),

    /// Parse url error
    #[error("Parsing url error: {0}")]
    ParseUrlError(url::ParseError),
}

impl From<mesc::MescError> for ParseError {
    fn from(err: mesc::MescError) -> Self {
        ParseError::MescError(err)
    }
}

/// Error performing a chunk operation
#[derive(Error, Debug)]
pub enum ChunkError {
    /// Error related to parsing
    #[error("Parsing error: {0}")]
    ChunkError(String),

    /// Error in chunk specification
    #[error("Block chunk not valid")]
    InvalidChunk,

    /// Error in creating a chunk stub
    #[error("Failed to create stub")]
    StubError,
}

/// Error related to file operations
#[derive(Error, Debug)]
pub enum FileError {
    /// Error in creating filepath
    #[error("Failed to build file path")]
    FilePathError(#[from] ChunkError),

    /// File path not given
    #[error("File path not given")]
    NoFilePathError(String),

    /// Error in writing file
    #[error("Error writing file")]
    FileWriteError,
}

/// Why a contract read (`eth_call`) produced no value.
///
/// The two cases look identical at the call site — both are an `Err` coming
/// back from the provider — but they mean opposite things about the chain, and
/// conflating them is how a run silently writes a column full of nulls while
/// reporting 100% success.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallOutcome {
    /// The node executed the call and the **contract** declined to answer:
    /// a revert, an invalid opcode, running out of gas, or an address that
    /// holds no code at all.
    ///
    /// This is a fact about the chain, not a failure of the extraction. "This
    /// address has no `totalSupply()`" is real, reportable data, so a null cell
    /// is the correct output and the row still counts as collected.
    ContractRefused,

    /// The **node** could not answer: state pruned behind a non-archive
    /// endpoint, a rate limit, a timeout, a dropped connection, an
    /// undecodable response.
    ///
    /// Nothing whatsoever is known about the contract here. Writing a null
    /// would fabricate the claim "we asked and there was nothing", so the row
    /// must surface as an error instead.
    NodeFailed,
}

/// JSON-RPC error code EIP-1474 reserves for `execution reverted`.
const EXECUTION_REVERTED_CODE: i64 = 3;

/// Lowercase fragments of the EVM's own execution-failure messages.
///
/// Every one of these is the EVM refusing to finish running *the contract*, so
/// the node did its job and the address has no answer to give. That is a
/// [`CallOutcome::ContractRefused`], and a null is the honest cell for it.
///
/// Taken from geth's `vm.Err*` set, which reth and erigon reproduce and which
/// providers pass through. The list was four fragments and covered a third of
/// that set; the rest were being classified as node failures and were failing
/// whole chunks. `contract_interfaces` is the most exposed dataset, because it
/// aims `eth_call` at arbitrary addresses holding arbitrary bytecode.
///
/// Anything not listed here still defaults to [`CallOutcome::NodeFailed`]. That
/// direction is the safe one: an unrecognised error surfaces loudly instead of
/// being laundered into a null.
const EVM_FAILURE_MESSAGES: &[&str] = &[
    "revert",
    "invalid opcode",
    "invalid jump",
    "out of gas",
    "stack underflow",
    "stack limit reached",
    "return data out of bounds",
    "max call depth exceeded",
    "gas uint64 overflow",
    "nonce uint64 overflow",
    "write protection",
    "invalid code",
    "max code size exceeded",
    "contract address collision",
];

impl CollectError {
    /// Classify this error as a contract-level refusal or a node-level failure.
    ///
    /// Only an `eth_call` that the node *ran to completion* can be a
    /// [`CallOutcome::ContractRefused`]; everything else — including the
    /// `-32602`/`-32000` families that non-archive providers return for pruned
    /// state — is a [`CallOutcome::NodeFailed`]. The default is deliberately
    /// `NodeFailed`: an unrecognised error must never be laundered into a null.
    pub fn call_outcome(&self) -> CallOutcome {
        let Self::ProviderError(rpc_err) = self else { return CallOutcome::NodeFailed };
        let Some(payload) = rpc_err.as_error_resp() else { return CallOutcome::NodeFailed };

        // A rate limit is the node throttling us, never the contract talking.
        // Checked before the message scan because some providers word their
        // throttle message in ways that could trip the substring match below.
        if payload.is_retry_err() {
            return CallOutcome::NodeFailed
        }

        // EIP-1474 reserves code 3 for "execution reverted", and geth uses it
        // for exactly that, carrying the revert blob in `data`. It is the one
        // structured signal available here, so it is checked before the
        // message: a provider that rewords the message still sets the code.
        if payload.code == EXECUTION_REVERTED_CODE {
            return CallOutcome::ContractRefused
        }

        // Otherwise the message is the only portable signal. Geth, reth,
        // erigon, nethermind and every provider that proxies them report
        // contract-level failures through it, not the code (which is an
        // un-namespaced -32000 on most of them). Matching on a message is
        // unlovely, but there is nothing else to match on.
        let message = payload.message.to_ascii_lowercase();
        let contract_refused =
            EVM_FAILURE_MESSAGES.iter().any(|fragment| message.contains(fragment));

        if contract_refused {
            CallOutcome::ContractRefused
        } else {
            CallOutcome::NodeFailed
        }
    }
}

/// Fold a contract read into `Option`, preserving node failures as errors.
///
/// This is the seam every `eth_call`-backed dataset must go through instead of
/// `.await.ok()`. `.ok()` maps *both* outcomes to `None`, which is what let a
/// pruned-state endpoint produce a full parquet file of nulls under a
/// "chunks errored: 0" banner.
///
/// # Errors
/// Returns the original error when it is a [`CallOutcome::NodeFailed`].
pub fn contract_read<T>(result: R<T>) -> R<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(e) => match e.call_outcome() {
            CallOutcome::ContractRefused => Ok(None),
            CallOutcome::NodeFailed => Err(e),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::rpc::json_rpc::ErrorPayload;
    use std::borrow::Cow;

    /// Build the `CollectError` shape a provider produces for a JSON-RPC error
    /// response, so the classifier is exercised on the real variant rather than
    /// a stand-in.
    fn error_resp(code: i64, message: &'static str) -> CollectError {
        CollectError::ProviderError(RpcError::ErrorResp(ErrorPayload {
            code,
            message: Cow::Borrowed(message),
            data: None,
        }))
    }

    #[test]
    fn plain_revert_is_a_contract_refusal() {
        // geth / reth wording for a bare `revert()`
        assert_eq!(
            error_resp(3, "execution reverted").call_outcome(),
            CallOutcome::ContractRefused
        );
    }

    #[test]
    fn revert_with_reason_is_a_contract_refusal() {
        assert_eq!(
            error_resp(-32000, "execution reverted: ERC20: zero address").call_outcome(),
            CallOutcome::ContractRefused
        );
    }

    #[test]
    fn invalid_opcode_is_a_contract_refusal() {
        // hitting a non-token address that happens to hold code
        assert_eq!(
            error_resp(-32000, "invalid opcode: INVALID").call_outcome(),
            CallOutcome::ContractRefused
        );
    }

    #[test]
    fn the_reserved_revert_code_is_enough_on_its_own() {
        // EIP-1474 reserves 3 for execution-reverted. A provider that rewords
        // the message still sets the code, so the code is checked first.
        assert_eq!(
            error_resp(3, "VM execution error.").call_outcome(),
            CallOutcome::ContractRefused
        );
    }

    #[test]
    fn every_evm_execution_failure_is_a_contract_refusal() {
        // The EVM refusing to finish running the contract, in geth's own
        // wording. Each of these used to be classified as a node failure and
        // failed the whole chunk instead of writing one null row. They arrive
        // under -32000, so only the message distinguishes them.
        for message in [
            "stack underflow (0 <=> 2)",
            "stack limit reached 1024 (1024)",
            "return data out of bounds",
            "max call depth exceeded",
            "gas uint64 overflow",
            "nonce uint64 overflow",
            "write protection",
            "invalid code: must not begin with 0xef",
            "max code size exceeded",
            "contract address collision",
        ] {
            assert_eq!(
                error_resp(-32000, message).call_outcome(),
                CallOutcome::ContractRefused,
                "{message}"
            );
        }
    }

    #[test]
    fn pruned_archive_state_is_a_node_failure() {
        // This is the regression that motivated the classifier: a non-archive
        // endpoint refusing historical state used to be laundered into a null
        // cell, so `erc20_supplies -b <old range>` wrote a file of nulls and
        // still reported "chunks errored: 0".
        assert_eq!(
            error_resp(-32602, "Archive requests require a personal token.").call_outcome(),
            CallOutcome::NodeFailed
        );
        assert_eq!(error_resp(-32000, "missing trie node").call_outcome(), CallOutcome::NodeFailed);
    }

    #[test]
    fn rate_limit_is_a_node_failure() {
        assert_eq!(error_resp(429, "Too Many Requests").call_outcome(), CallOutcome::NodeFailed);
        assert_eq!(
            error_resp(-32005, "exceeded project rate limit").call_outcome(),
            CallOutcome::NodeFailed
        );
    }

    #[test]
    fn unrecognised_errors_default_to_node_failure() {
        // The default must never be `ContractRefused`: an unclassified error
        // turning into a null is exactly the silent-data-loss bug.
        assert_eq!(error_resp(-32603, "internal error").call_outcome(), CallOutcome::NodeFailed);
        assert_eq!(CollectError::BadSchemaError.call_outcome(), CallOutcome::NodeFailed);
        assert_eq!(err("something went sideways").call_outcome(), CallOutcome::NodeFailed);
    }

    #[test]
    fn contract_read_keeps_a_successful_value() {
        assert_eq!(contract_read(Ok::<u8, CollectError>(7)).unwrap(), Some(7));
    }

    #[test]
    fn contract_read_folds_a_revert_into_none() {
        let out = contract_read(Err::<u8, _>(error_resp(3, "execution reverted"))).unwrap();
        assert_eq!(out, None);
    }

    #[test]
    fn contract_read_propagates_a_node_failure() {
        let out =
            contract_read(Err::<u8, _>(error_resp(-32602, "Archive requests require a token")));
        assert!(out.is_err(), "a node failure must surface as an error, never as a null cell");
    }
}
