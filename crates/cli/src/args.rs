use clap_cryo::Parser;
use color_print::cstr;
use colored::Colorize;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{default::Default, path::PathBuf};

/// Command line arguments
#[derive(Parser, Debug, Serialize, Deserialize, Clone, Default)]
#[command(
    name = "triodion",
    author,
    version = triodion_core::TRIODION_VERSION,
    about = &get_about_str(),
    long_about = None,
    styles=get_styles(),
    after_help=&get_after_str(),
    allow_negative_numbers = true,
)]
pub struct Args {
    /// Datatype to collect
    #[arg(help=get_datatype_help(), num_args(0..))]
    pub datatype: Vec<String>,

    /// Block numbers, see syntax below
    #[arg(short, long, allow_negative_numbers = true, help_heading = "Content Options", num_args(1..))]
    pub blocks: Option<Vec<String>>,

    /// Timestamps in unix, see syntax below
    #[arg(long, allow_negative_numbers = true, help_heading = "Content Options", num_args(0..))]
    pub timestamps: Option<Vec<String>>,

    /// Transaction hashes, see syntax below
    #[arg(
        short,
        long,
        help_heading = "Content Options",
        num_args(1..),
    )]
    pub txs: Option<Vec<String>>,

    /// Align chunk boundaries to regular intervals,
    /// e.g. (1000 2000 3000), not (1106 2106 3106)
    #[arg(short, long, help_heading = "Content Options", verbatim_doc_comment)]
    pub align: bool,

    /// Reorg buffer, save blocks only when this old,
    /// can be a number of blocks
    #[arg(
        long,
        default_value_t = 0,
        value_name = "N_BLOCKS",
        help_heading = "Content Options",
        verbatim_doc_comment
    )]
    pub reorg_buffer: u64,

    /// Columns to include alongside the defaults,
    /// use `all` to include all available columns
    #[arg(short, long, value_name="COLS", num_args(0..), verbatim_doc_comment, help_heading="Content Options")]
    pub include_columns: Option<Vec<String>>,

    /// Columns to exclude from the defaults
    #[arg(short, long, value_name="COLS", num_args(0..), help_heading="Content Options")]
    pub exclude_columns: Option<Vec<String>>,

    /// Columns to use instead of the defaults,
    /// use `all` to use all available columns
    #[arg(long, value_name="COLS", num_args(0..), verbatim_doc_comment, help_heading="Content Options")]
    pub columns: Option<Vec<String>>,

    /// Set output datatype(s) of U256 integers
    /// [default: binary, string, f64]
    #[arg(long, num_args(1..), help_heading = "Content Options", verbatim_doc_comment)]
    pub u256_types: Option<Vec<String>>,

    /// Use hex string encoding for binary columns
    #[arg(long, help_heading = "Content Options")]
    pub hex: bool,

    /// Columns(s) to sort by, `none` for unordered
    #[arg(short, long, num_args(0..), help_heading="Content Options")]
    pub sort: Option<Vec<String>>,

    /// Exclude items from failed transactions
    #[arg(long, help_heading = "Content Options")]
    pub exclude_failed: bool,

    /// RPC url [default: 1. MESC 2. ETH_RPC_URL]
    #[arg(short, long, help_heading = "Source Options")]
    pub rpc: Option<String>,

    /// L1 (settlement) RPC url, reserved for L2 datasets that read L1-side events.
    ///
    /// No dataset reads it yet. It connects and reports its chain id so the
    /// plumbing stays exercised, and is otherwise inert. The help text here
    /// used to name six such datasets, none of which exist.
    #[arg(long, value_name = "URL", help_heading = "Source Options")]
    pub l1_rpc: Option<String>,

    /// Consensus-layer (beacon) REST API url, e.g. http://localhost:5052.
    ///
    /// Required by the beacon datasets. The slot clock — genesis time, seconds
    /// per slot, the blob schedule — is read from this node rather than
    /// compiled in, so there is no default and no guess.
    #[arg(long, value_name = "URL", help_heading = "Source Options")]
    pub beacon_rpc: Option<String>,

    /// Blob archive url, for slots the beacon node has pruned.
    ///
    /// Beacon nodes only serve blob sidecars for ~18 days. Pass an archive to
    /// reach older blobs; `--blob-archive default` uses the public Blobscan
    /// API. Without one, historical blob queries error rather than silently
    /// reporting a block as blob-free.
    ///
    /// Usable on its own, without `--beacon-rpc`: an archive is keyed by
    /// execution block number and reports each blob's slot itself, so it needs
    /// no slot clock. It cannot serve blob bytes, only their commitments.
    #[arg(long, value_name = "URL", help_heading = "Source Options")]
    pub blob_archive: Option<String>,

    /// Network name [default: name of eth_getChainId]
    #[arg(long, help_heading = "Source Options")]
    pub network_name: Option<String>,

    /// Ratelimit on requests per second
    #[arg(short('l'), long, value_name = "limit", help_heading = "Acquisition Options")]
    pub requests_per_second: Option<u32>,

    /// Max retries for provider errors
    #[arg(long, default_value_t = 5, value_name = "R", help_heading = "Acquisition Options")]
    pub max_retries: u32,

    /// Initial retry backoff time (ms)
    #[arg(long, default_value_t = 500, value_name = "B", help_heading = "Acquisition Options")]
    pub initial_backoff: u64,

    /// The number of compute units per second for this provider
    #[arg(long, default_value_t = 50, value_name = "U", help_heading = "Acquisition Options")]
    pub compute_units_per_second: u64,

    /// Global number of concurrent requests
    #[arg(long, value_name = "M", help_heading = "Acquisition Options")]
    pub max_concurrent_requests: Option<u64>,

    /// Number of chunks processed concurrently
    #[arg(long, value_name = "M", help_heading = "Acquisition Options")]
    pub max_concurrent_chunks: Option<u64>,

    /// Chunk collection order (normal, reverse, random)
    #[arg(long, help_heading = "Acquisition Options")]
    pub chunk_order: Option<String>,

    /// Dry run, collect no data
    #[arg(short, long, help_heading = "Acquisition Options")]
    pub dry: bool,

    /// Remember current command for future use
    #[arg(long)]
    pub remember: bool,

    /// Extra verbosity
    #[arg(short, long)]
    pub verbose: bool,

    /// Run quietly without printing information to stdout
    #[arg(long)]
    pub no_verbose: bool,

    /// Number of blocks per file
    #[arg(short, long, default_value_t = 1000, help_heading = "Output Options")]
    pub chunk_size: u64,

    /// Number of files (alternative to --chunk-size)
    #[arg(long, help_heading = "Output Options")]
    pub n_chunks: Option<u64>,

    /// Dimensions to partition by
    #[arg(long, help_heading = "Output Options")]
    pub partition_by: Option<Vec<String>>,

    /// Directory for output files
    #[arg(short, long, default_value = ".", help_heading = "Output Options")]
    pub output_dir: String,

    /// Subdirectories for output files
    /// can be `datatype`, `network`, or custom string
    #[arg(long, help_heading = "Output Options", verbatim_doc_comment, num_args(1..))]
    pub subdirs: Vec<String>,

    /// Label to add to each filename
    #[arg(long, help_heading = "Output Options")]
    pub label: Option<String>,

    /// Overwrite existing files instead of skipping
    #[arg(long, help_heading = "Output Options")]
    pub overwrite: bool,

    /// Save as csv instead of parquet
    #[arg(long, help_heading = "Output Options")]
    pub csv: bool,

    /// Save as json instead of parquet
    #[arg(long, help_heading = "Output Options")]
    pub json: bool,

    /// Number of rows per row group in parquet file
    #[arg(long, value_name = "GROUP_SIZE", help_heading = "Output Options")]
    pub row_group_size: Option<usize>,

    /// Number of rows groups in parquet file
    #[arg(long, help_heading = "Output Options")]
    pub n_row_groups: Option<usize>,

    /// Do not write statistics to parquet files
    #[arg(long, help_heading = "Output Options")]
    pub no_stats: bool,

    /// Compression algorithm and level
    #[arg(long, help_heading="Output Options", value_name="NAME [#]", num_args(1..=2), default_value = "lz4")]
    pub compression: Vec<String>,

    /// Directory to save summary report
    /// [default: {output_dir}/.triodion/reports]
    #[arg(long, help_heading = "Output Options", verbatim_doc_comment)]
    pub report_dir: Option<PathBuf>,

    /// Avoid saving a summary report
    #[arg(long, help_heading = "Output Options")]
    pub no_report: bool,

    /// Address(es)
    #[arg(long, help_heading = "Dataset-specific Options", num_args(1..))]
    pub address: Option<Vec<String>>,

    /// To Address(es)
    #[arg(long, help_heading = "Dataset-specific Options", value_name="address", num_args(1..))]
    pub to_address: Option<Vec<String>>,

    /// From Address(es)
    #[arg(long, help_heading = "Dataset-specific Options", value_name="address", num_args(1..))]
    pub from_address: Option<Vec<String>>,

    /// Call data(s) to use for eth_calls
    #[arg(long, help_heading = "Dataset-specific Options", num_args(1..))]
    pub call_data: Option<Vec<String>>,

    /// Function(s) to use for eth_calls
    #[arg(long, help_heading = "Dataset-specific Options", num_args(1..))]
    pub function: Option<Vec<String>>,

    /// Input(s) to use for eth_calls
    #[arg(long, help_heading = "Dataset-specific Options", num_args(1..))]
    pub inputs: Option<Vec<String>>,

    /// Slot(s)
    #[arg(long, help_heading = "Dataset-specific Options", num_args(1..))]
    pub slot: Option<Vec<String>>,

    /// Contract address(es)
    #[arg(long, help_heading = "Dataset-specific Options", num_args(1..))]
    pub contract: Option<Vec<String>>,

    /// Topic0(s)
    #[arg(long, visible_alias = "event", help_heading = "Dataset-specific Options", num_args(1..))]
    pub topic0: Option<Vec<String>>,

    /// Topic1(s)
    #[arg(long, help_heading = "Dataset-specific Options", num_args(1..))]
    pub topic1: Option<Vec<String>>,

    /// Topic2(s)
    #[arg(long, help_heading = "Dataset-specific Options", num_args(1..))]
    pub topic2: Option<Vec<String>>,

    /// Topic3(s)
    #[arg(long, help_heading = "Dataset-specific Options", num_args(1..))]
    pub topic3: Option<Vec<String>>,

    /// Event signature for log decoding
    #[arg(long, value_name = "SIG", help_heading = "Dataset-specific Options", num_args(1..))]
    pub event_signature: Option<String>,

    /// Blocks per request (eth_getLogs)
    #[arg(
        long,
        value_name = "BLOCKS",
        default_value_t = 1,
        help_heading = "Dataset-specific Options"
    )]
    pub inner_request_size: u64,

    /// Event signature for log decoding
    #[arg(long, value_name = "tracer", help_heading = "Dataset-specific Options")]
    pub js_tracer: Option<String>,

    /// Disable Multicall3 batching for eth_calls / erc20_balances.
    ///
    /// Multicall3 batching is **on by default** — calls sharing a block are
    /// aggregated through Multicall3 in chunks of `multicall_batch_size`, with
    /// a halving fallback on RPC error and a per-call fallback at blocks
    /// earlier than the Multicall3 deploy block on the active chain. Pass
    /// `--no-multicall` to fall back to one `eth_call` per call.
    #[arg(
        long = "no-multicall",
        action = clap_cryo::ArgAction::SetFalse,
        default_value_t = true,
        help_heading = "Dataset-specific Options"
    )]
    pub multicall: bool,

    /// Backwards-compatibility no-op for the legacy `--multicall` flag.
    ///
    /// Multicall3 batching is on by default now (see `--no-multicall`); the old
    /// `--multicall` flag is accepted silently so existing scripts don't break.
    /// This field is not read.
    #[arg(
        long = "multicall",
        action = clap_cryo::ArgAction::SetTrue,
        hide = true,
        help_heading = "Dataset-specific Options"
    )]
    pub _multicall_legacy_alias: bool,

    /// Cap on inner eth_calls per Multicall3 batch (0 = the dataset's own default).
    ///
    /// This caps INNER CALLS, not rows: rows per batch is this value divided by
    /// the dataset's calls-per-row (`erc20_metadata` issues 3, so N=250 ships ~83
    /// rows per multicall). Leave at 0 to get `DEFAULT_MULTICALL_BATCH_SIZE` — or
    /// whatever a dataset overrides it to for expensive inner calls. A non-zero
    /// value overrides every dataset's preference.
    #[arg(long, value_name = "N", default_value_t = 0, help_heading = "Dataset-specific Options")]
    pub multicall_batch_size: u32,

    /// Mark the whole batch as failed if any inner call reverts (default: per-call failures return
    /// null)
    #[arg(long, help_heading = "Dataset-specific Options")]
    pub multicall_require_success: bool,

    /// Disable `eth_call` state-override reads for `slots` / `balances` / `proxy_slots`.
    ///
    /// On by default. Replaces the target's code with a small extractor loop
    /// that answers a whole batch of slots (or balances) in one request,
    /// instead of one `eth_getStorageAt` / `eth_getBalance` per row.
    ///
    /// Turn this off for an endpoint you do not trust to honour the third
    /// `eth_call` parameter. Plain JSON-RPC batching is a separate switch
    /// (`--no-batch-rpc-calls`) because it asks nothing of the node.
    ///
    /// A batch that fails falls back to one request per row, so this switch
    /// cannot change results — only how many requests they cost.
    #[arg(
        long = "no-batch-state-reads",
        action = clap_cryo::ArgAction::SetFalse,
        default_value_t = true,
        help_heading = "Dataset-specific Options"
    )]
    pub batch_state_reads: bool,

    /// Slots / addresses per state-override call (0 = the dataset's own default).
    ///
    /// Applies to the extractor path only (`slots`, `balances`, `proxy_slots`); the JSON-RPC
    /// batch path used by `blocks`, `codes` and `nonces` negotiates its own size
    /// with the provider.
    ///
    /// The default is 1000, chosen from measurement rather than the gas ceiling:
    /// 10,000 slots in one call works but a 20,000-slot request is refused for
    /// payload size, and ten parallel 1000-slot calls finish sooner than one
    /// 10,000-slot call. Raise it when pointing at your own node.
    #[arg(long, value_name = "N", default_value_t = 0, help_heading = "Dataset-specific Options")]
    pub state_override_batch_size: u32,

    /// Disable JSON-RPC request batching for `blocks` / `nonces` / `codes`.
    ///
    /// On by default. Packs many identical calls into one HTTP request — 100
    /// `eth_getBlockByNumber` calls in one envelope rather than 100 requests.
    /// It needs nothing from the node beyond JSON-RPC itself, which is why it
    /// is a separate switch from `--no-batch-state-reads`.
    ///
    /// The batch size negotiates itself downward when a provider refuses one
    /// (OP Mainnet answers `413` above ten calls), and a batch that still fails
    /// falls back to one request per row. Turn this off only for an endpoint
    /// that mishandles batch envelopes outright.
    #[arg(
        long = "no-batch-rpc-calls",
        action = clap_cryo::ArgAction::SetFalse,
        default_value_t = true,
        help_heading = "Dataset-specific Options"
    )]
    pub batch_rpc_calls: bool,
}

impl Args {
    pub(crate) fn merge_with_precedence(self, other: Args) -> Self {
        let default_struct = Args::default();

        let mut s1_value: Value = serde_json::to_value(self).expect("Failed to serialize to JSON");
        let s2_value: Value = serde_json::to_value(other).expect("Failed to serialize to JSON");
        let default_value: Value =
            serde_json::to_value(default_struct).expect("Failed to serialize to JSON");

        if let (Value::Object(s1_map), Value::Object(s2_map), Value::Object(default_map)) =
            (&mut s1_value, &s2_value, &default_value)
        {
            for (k, v) in s2_map.iter() {
                // If the value in s2 is different from the default, overwrite the value in s1
                if default_map.get(k) != Some(v) {
                    s1_map.insert(k.clone(), v.clone());
                }
            }
        }

        serde_json::from_value(s1_value).expect("Failed to deserialize from JSON")
    }
}

pub(crate) fn get_styles() -> clap_cryo::builder::Styles {
    let white = anstyle::Color::Rgb(anstyle::RgbColor(255, 255, 255));
    let green = anstyle::Color::Rgb(anstyle::RgbColor(0, 225, 0));
    let grey = anstyle::Color::Rgb(anstyle::RgbColor(170, 170, 170));
    let title = anstyle::Style::new().bold().fg_color(Some(green));
    let arg = anstyle::Style::new().bold().fg_color(Some(white));
    let comment = anstyle::Style::new().fg_color(Some(grey));
    clap_cryo::builder::Styles::styled()
        .header(title)
        .error(comment)
        .usage(title)
        .literal(arg)
        .placeholder(comment)
        .valid(title)
        .invalid(comment)
}

fn get_about_str() -> String {
    cstr!(
        r#"<white><bold>triodion</bold></white> extracts blockchain data to parquet, csv, or json"#
    )
    .to_string()
}

fn get_after_str() -> String {
    let header = "Optional Subcommands:".truecolor(0, 225, 0).bold().to_string();
    let subcommands = cstr!(
        r#"
      <white><bold>triodion help</bold></white>                      display help message
      <white><bold>triodion help syntax</bold></white>               display block + tx specification syntax
      <white><bold>triodion help datasets</bold></white>             display list of all datasets
      <white><bold>triodion help</bold></white>"#
    );
    let post_subcommands = " <DATASET(S)>         display info about a dataset";
    format!("{}{}{}", header, subcommands, post_subcommands)
}

fn get_datatype_help() -> &'static str {
    cstr!(
        r#"datatype(s) to collect, use <white><bold>triodion datasets</bold></white> to see all available"#
    )
}
