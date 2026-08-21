# CLI Reference

```
triodion [OPTIONS] [DATATYPE]...
```

`DATATYPE` is one or more dataset names, or a dataset group name. Run
`triodion datasets` to list them. Several datasets can be collected in one
command:

```bash
triodion blocks transactions logs -b 18M:18.1M
```

Every option below is reproduced from `triodion help`. Where the binary prints
a default it is repeated here exactly.

Four options print no default: `--requests-per-second`,
`--max-concurrent-requests`, `--max-concurrent-chunks` and `--chunk-order`.
Their Default column describes what happens when the flag is absent, taken from
the source rather than from the help text.

## General

| Option | Meaning |
| :- | :- |
| `--remember` | Save this command as the default for the current directory. A later `triodion` run that names no datatype repeats it |
| `-v`, `--verbose` | Extra verbosity |
| `--no-verbose` | Print nothing to stdout |
| `-h`, `--help` | Print help. `--help` prints the long form |
| `-V`, `--version` | Print version |

## Content Options

These decide which rows and which columns end up in the output.

| Option | Default | Meaning |
| :- | :- | :- |
| `-b`, `--blocks <BLOCKS>...` | | Block numbers or ranges. See `triodion help syntax` |
| `--timestamps [<TIMESTAMPS>...]` | | Unix timestamps instead of block numbers. Each is resolved by binary search to the closest block at or before it |
| `-t`, `--txs <TXS>...` | | Transaction hashes, or a parquet file of them |
| `-a`, `--align` | off | Align chunk boundaries to round intervals: `1000 2000 3000`, not `1106 2106 3106` |
| `--reorg-buffer <N_BLOCKS>` | `0` | Write a block only once it is this many blocks old. Guards against reorgs at the chain tip |
| `-i`, `--include-columns [<COLS>...]` | | Add these columns to the defaults. `all` adds every available column |
| `-e`, `--exclude-columns [<COLS>...]` | | Drop these columns from the defaults |
| `--columns [<COLS>...]` | | Use exactly these columns instead of the defaults. `all` uses every available column |
| `--u256-types <U256_TYPES>...` | `binary, string, f64` | Which representations each `u256` column is emitted as |
| `--hex` | off | Encode binary columns as hex strings |
| `-s`, `--sort [<SORT>...]` | dataset's own sort | Columns to sort by. `none` leaves rows unordered |
| `--exclude-failed` | off | Drop items belonging to failed transactions |

A `u256` column named `value` becomes one column per requested type:
`value_binary`, `value_string`, `value_f64`, and likewise for `f32`, `u32`,
`u64` and `d128`.

## Source Options

| Option | Default | Meaning |
| :- | :- | :- |
| `-r`, `--rpc <RPC>` | MESC, then `ETH_RPC_URL` | Execution-layer JSON-RPC url |
| `--l1-rpc <URL>` | `L1_RPC_URL`, else unset | A second execution provider, for L2 runs that read L1-side data |
| `--beacon-rpc <URL>` | `BEACON_RPC_URL`, else unset | Consensus-layer REST API, for example `http://localhost:5052`. Required by the beacon datasets unless `--blob-archive` is given |
| `--blob-archive <URL>` | `BLOB_ARCHIVE_URL`, else unset | Blob archive for slots the beacon node has pruned. The value `default` selects `https://api.blobscan.com` |
| `--network-name <NETWORK_NAME>` | name of the chain id the node reports | Name used in output filenames |

`--blob-archive` is a fallback inside the consensus-layer client, and it also
works on its own: an archive is keyed by execution block number and reports
each blob's slot itself, so it needs no slot clock. It cannot serve blob bytes,
so an archive-only run leaves `blob` and `epoch` null. See
[Data Sources](../../intro/data_sources.md#the-consensus-layer) for the
retention window that makes it necessary.

## Acquisition Options

These control how hard triodion drives the node. The defaults suit a local
node. A third-party provider usually needs `--requests-per-second` and
`--max-concurrent-requests` set explicitly.

| Option | Default | Meaning |
| :- | :- | :- |
| `-l`, `--requests-per-second <limit>` | unlimited | Rate limit on outgoing requests |
| `--max-retries <R>` | `5` | Retries per provider error |
| `--initial-backoff <B>` | `500` | Retry backoff, in milliseconds. Flat, not exponential: each retry waits this long, or the server's own backoff hint if it sent one, plus a compute-budget offset |
| `--compute-units-per-second <U>` | `50` | Compute-unit budget, for providers that price by compute unit |
| `--max-concurrent-requests <M>` | `100` | Global cap on requests in flight. `0` means no limit |
| `--max-concurrent-chunks <M>` | `4` | Cap on chunks processed at once. `0` means no limit |
| `--chunk-order <CHUNK_ORDER>` | `normal` | Order chunks are collected in: `normal`, `reverse` or `random` |
| `-d`, `--dry` | off | Print the schema and the planned work, then stop. Collects nothing |

JSON-RPC batches are sized adaptively. When a provider rejects a batch for
being too large, triodion splits it and retries. Rate limits and
authentication failures are excluded from that behaviour, because shrinking a
batch sends more requests to a node that just asked for fewer.

## Output Options

| Option | Default | Meaning |
| :- | :- | :- |
| `-c`, `--chunk-size <CHUNK_SIZE>` | `1000` | Blocks per output file |
| `--n-chunks <N_CHUNKS>` | | Number of files to produce, instead of a fixed chunk size |
| `--partition-by <PARTITION_BY>` | | Dimensions to split files by, for example `block`, `address`, `topic0` |
| `-o`, `--output-dir <OUTPUT_DIR>` | `.` | Directory for output files |
| `--subdirs <SUBDIRS>...` | | Subdirectories to nest output in: `datatype`, `network`, or a literal string |
| `--label <LABEL>` | | Label inserted into every filename |
| `--overwrite` | off | Rewrite existing files instead of skipping them |
| `--csv` | off | Write csv instead of parquet |
| `--json` | off | Write json instead of parquet |
| `--row-group-size <GROUP_SIZE>` | | Rows per parquet row group |
| `--n-row-groups <N_ROW_GROUPS>` | | Number of row groups per parquet file |
| `--no-stats` | off | Omit statistics from parquet files |
| `--compression <NAME [#]>...` | `lz4` | Compression algorithm, and level where the algorithm takes one |
| `--report-dir <REPORT_DIR>` | `{output_dir}/.triodion/reports` | Directory for the run's summary report |
| `--no-report` | off | Write no summary report |

Existing files are skipped by default, so a repeated command fills in only what
is missing.

## Dataset-specific Options

Each of these applies to the datasets that declare it. `triodion help <DATASET>`
lists the parameters a given dataset accepts, and which of them are required.

| Option | Default | Meaning |
| :- | :- | :- |
| `--address <ADDRESS>...` | | Address(es) |
| `--to-address <address>...` | | To address(es) |
| `--from-address <address>...` | | From address(es) |
| `--call-data <CALL_DATA>...` | | Raw call data for `eth_calls` |
| `--function <FUNCTION>...` | | Function(s) for `eth_calls`, used with `--inputs` |
| `--inputs <INPUTS>...` | | Argument(s) for `--function` |
| `--slot <SLOT>...` | | Storage slot(s) |
| `--contract <CONTRACT>...` | | Contract address(es) |
| `--topic0 <TOPIC0>...` | | First log topic. Alias `--event` |
| `--topic1 <TOPIC1>...` | | Second log topic |
| `--topic2 <TOPIC2>...` | | Third log topic |
| `--topic3 <TOPIC3>...` | | Fourth log topic |
| `--event-signature <SIG>...` | | Event signature used to decode logs into typed columns |
| `--inner-request-size <BLOCKS>` | `1` | Blocks per `eth_getLogs` request |
| `--js-tracer <tracer>` | | The JavaScript tracer that `javascript_traces` runs. That dataset errors without it |
| `--no-multicall` | off | Send one `eth_call` per row instead of batching through Multicall3 |
| `--multicall-batch-size <N>` | `0` | Cap on inner calls per Multicall3 batch. `0` means the dataset's own default |
| `--multicall-require-success` | off | Fail the whole batch if any inner call reverts. By default a reverting call returns null and the rest survive |

Multicall3 batching is on by default for `eth_calls` and the erc20 and erc721
read datasets. It halves the batch on an RPC error, and falls back to
individual calls at blocks earlier than the Multicall3 deploy block on the
active chain.

## Subcommands

| Command | Prints |
| :- | :- |
| `triodion help` | The help message |
| `triodion help syntax` | Block and transaction specification syntax |
| `triodion help datasets` | Every dataset and dataset group |
| `triodion help <DATASET(S)>` | A dataset's schema, sort order, and required and optional parameters |
