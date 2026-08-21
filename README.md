# triodion

[![Rust](https://github.com/KonScanner/triodion/actions/workflows/build_and_test.yml/badge.svg)](https://github.com/KonScanner/triodion/actions/workflows/build_and_test.yml)

> **triodion** is a modified fork of [cryo](https://github.com/paradigmxyz/cryo), originally
> developed by Paradigm and the Cryo Contributors. It is not affiliated with, nor endorsed
> by, Paradigm. It diverges from upstream commit `559b654` (2025-01-08) and was renamed
> from `cryo` to `triodion` to avoid namespace collision. See [NOTICE](NOTICE) for the
> statement of changes required by Apache-2.0 section 4(b). Licensed MIT OR Apache-2.0.

`triodion` is the easiest way to extract blockchain data to parquet, csv, json, or a python dataframe.

`triodion` is also extremely flexible, with [many different options](#triodion-help) to control how data is extracted + filtered + formatted

*`triodion` is an early WIP, please report bugs + feedback to the issue tracker*

*note that `triodion`'s default settings will slam a node too hard for use with 3rd party RPC providers. Instead, `--requests-per-second` and `--max-concurrent-requests` should be used to impose ratelimits. Such settings will be handled automatically in a future release*.

## Contents

1. [Example Usage](#example-usage)
2. [Installation](#installation)
3. [Data Schema](#data-schemas)
4. [Multi-Chain Support](#multi-chain-support)
5. [Beacon Chain: Blobs and Execution Requests](#beacon-chain-blobs-and-execution-requests)
6. [Breaking Changes](#breaking-changes)
7. [Code Guide](#code-guide)
8. [Documentation](#documentation)
    1. [Basics](#triodion-help)
    2. [Syntax](#triodion-syntax)
    3. [Datasets](#triodion-datasets)

## Example Usage

use as `triodion <dataset> [OPTIONS]`

| Example | Command |
| :- | :- |
| Extract all logs from block 16,000,000 up to but not including 17,000,000 | `triodion logs -b 16M:17M` |
| Extract blocks, transactions and traces missing from current directory | `triodion blocks txs traces` |
| Extract to csv instead of parquet | `triodion blocks txs traces --csv` |
| Extract only certain columns | `triodion blocks --columns block_number timestamp` |
| Add optional columns to the defaults | `triodion blocks -i blob_gas_used excess_blob_gas` |
| Drop a column you do not want | `triodion blocks txs -e chain_id` |
| Dry run to view output schemas or expected work | `triodion storage_diffs --dry` |
| Extract all USDC events | `triodion logs --contract 0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48` |

For a more complex example, see the [Uniswap Example](./examples/uniswap.sh).

`triodion` uses `ETH_RPC_URL` env var as the data source unless `--rpc <url>` is given

## Installation

The simplest way to use `triodion` is as a cli tool:

#### Method 1: install from source

```bash
git clone https://github.com/KonScanner/triodion
cd triodion
cargo install --path ./crates/cli
```

This method requires having rust installed. See [rustup](https://rustup.rs/) for instructions.

#### Method 2: install from git

```bash
cargo install --git https://github.com/KonScanner/triodion triodion-cli
```

The `triodion-*` crates are not published to crates.io. Install from git or from source.

This method requires having rust installed. See [rustup](https://rustup.rs/) for instructions.

Make sure that `~/.cargo/bin` is on your `PATH`. One way to do this is by adding the line `export PATH="$HOME/.cargo/bin:$PATH"` to your `~/.bashrc` or `~/.profile`.

### Python Installation

`triodion` can also be installed as a python package:

#### Installing the python package from pypi

(make sure rust is installed first, see [rustup](https://www.rust-lang.org/tools/install))

```bash
pip install maturin
pip install triodion
```

```python
import triodion
```

#### Installing the python package from source

```bash
pip install maturin
git clone https://github.com/KonScanner/triodion
cd triodion/crates/python
maturin build --release
pip install --force-reinstall <OUTPUT_OF_MATURIN_BUILD>.whl
```

## Data Schemas

Many `triodion` cli options will affect output schemas by adding/removing columns or changing column datatypes.

`triodion` will always print out data schemas before collecting any data. To view these schemas without collecting data, use `--dry` to perform a dry run.

#### Schema Design Guide

An attempt is made to ensure that the dataset schemas conform to a common set of design guidelines:
- By default, rows should contain enough information in their columns to be order-able (unless the rows do not have an intrinsic order).
- Columns should usually be named by their JSON-RPC or ethers.rs defaults, except in cases where a much more explicit name is available.
- To make joins across tables easier, a given piece of information should use the same datatype and column name across tables when possible.
- Large ints such as `u256` should allow multiple conversions. A `value` column of type `u256` should allow: `value_binary`, `value_string`, `value_f32`, `value_f64`, `value_u32`, `value_u64`, and `value_d128`. These types can be specified at runtime using the `--u256-types` argument.
- By default, columns related to non-identifying cryptographic signatures are omitted by default. For example, `state_root` of a block or `v`/`r`/`s` of a transaction.
- Integer values that can never be negative should be stored as unsigned integers.
- Every table should allow a `chain_id` column so that data from multiple chains can be easily stored in the same table.

Standard types across tables:
- `block_number`: `u32`
- `transaction_index`: `u32` on the log- and trace-derived datasets, `u64` on `transactions`, `blobs`, `access_lists`, `authorizations` and the `geth_*` diff datasets
- `nonce`: `u64`
- `gas_used`: `u64`
- `gas_limit`: `u64`
- `chain_id`: `u64`
- `timestamp`: `u32`

#### JSON-RPC

`triodion` obtains almost all of its data using the [JSON-RPC](https://ethereum.org/en/developers/docs/apis/json-rpc/) protocol standard. The beacon datasets are the exception: they read the consensus-layer REST API instead. See [Beacon Chain and Blobs](#beacon-chain-and-blobs).

|dataset|blocks per request|results per block|method|
|-|-|-|-|
|Blocks|1|1|`eth_getBlockByNumber`|
|Transactions|1|multiple|`eth_getBlockByNumber`, `eth_getBlockReceipts`, `eth_getTransactionReceipt`|
|Logs|multiple|multiple|`eth_getLogs`|
|Contracts|1|multiple|`trace_block`|
|Traces|1|multiple|`trace_block`|
|State Diffs|1|multiple|`trace_replayBlockTransactions`|
|Vm Traces|1|multiple|`trace_replayBlockTransactions`|
|Blobs|1|multiple|`eth_getBlockByNumber`, beacon `/eth/v1/beacon/blob_sidecars/{slot}`|

`triodion` uses [alloy](https://github.com/alloy-rs/alloy) to perform JSON-RPC requests, over alloy's `AnyNetwork` rather than its `Ethereum` network, so that a transaction type byte Ethereum does not define cannot fail a whole block. See [Multi-Chain Support](#multi-chain-support) for the chain families this covers.

A future version of `triodion` will be able to bypass JSON-RPC and query node data directly.

## Multi-Chain Support

`triodion` targets three EVM chain families: Ethereum mainnet, the OP stack (OP Mainnet, Base, and the other OP-stack rollups), and the Arbitrum stack (Arbitrum One, Arbitrum Nova).

The three families do not agree on the set of EIP-2718 transaction type bytes:

| family | transaction type bytes |
| :- | :- |
| Ethereum | `0x00` legacy, `0x01` EIP-2930, `0x02` EIP-1559, `0x03` EIP-4844, `0x04` EIP-7702 |
| OP stack | `0x7e` deposit |
| Arbitrum | `0x64` deposit, `0x65` unsigned, `0x66` contract, `0x68` retry, `0x69` submit retryable, `0x6a` internal, `0x78` legacy |

`0x67` is deliberately unassigned in the Arbitrum fork of go-ethereum, so it is absent from the table.

Support for the two L2 families is not cosmetic. alloy's `TxEnvelope` models Ethereum's type bytes only, and its `serde` implementation is an untagged enum, so one unrecognised type byte fails the whole `eth_getBlockByNumber` response with "data did not match any variant of untagged enum BlockTransactions". Every OP-stack block opens with an L1-attributes deposit (`0x7e`) and every Arbitrum block with an ArbOS internal transaction (`0x6a`). Every block therefore failed, and `transactions` collected zero rows on OP Mainnet, Base and Arbitrum One.

`triodion` speaks alloy's `AnyNetwork` instead of `Ethereum`. An unknown type byte keeps every JSON field verbatim, so the shared columns (`nonce`, `value`, `input`, the gas fields, `to_address`, `from_address`) are read the same way on every chain, and the family-specific fields become opt-in columns. The `chain_family` column on `transactions` records which family defined the type byte: `ethereum`, `op_stack`, `arbitrum`, or `unknown`.

Measured after the fix, with default columns and zero errors:

| chain | blocks | transaction rows |
| :- | -: | -: |
| Ethereum mainnet (local archive node) | 3 | 311 |
| OP Mainnet | 2 | 124 |
| Arbitrum One | 2 | 13 |
| Base | 2 | 2,034 |

The Base figure counts transaction bodies. Its free public endpoint ratelimits the receipt fetch, which is a limit of that endpoint and not of `triodion`. Use `--requests-per-second` or a private endpoint to collect the receipt-derived columns.

#### Worked examples

Each example reads the two most recent blocks. Public endpoints need a ratelimit, as noted at the top of this file.

```bash
# OP Mainnet
triodion transactions --rpc https://mainnet.optimism.io --blocks -2:latest \
    --requests-per-second 5

# Base
triodion transactions --rpc https://mainnet.base.org --blocks -2:latest \
    --requests-per-second 5

# Arbitrum One
triodion transactions --rpc https://arb1.arbitrum.io/rpc --blocks -2:latest \
    --requests-per-second 5
```

The family-specific columns are opt-in. Add them with `--include-columns`:

```bash
# OP deposit fields and the L1-fee receipt family
triodion transactions --rpc https://mainnet.optimism.io --blocks -2:latest \
    --requests-per-second 5 \
    --include-columns chain_family source_hash mint l1_fee l1_gas_used

# Arbitrum's split of gas between L1 data availability and L2 execution
triodion transactions --rpc https://arb1.arbitrum.io/rpc --blocks -2:latest \
    --requests-per-second 5 \
    --include-columns chain_family gas_used_for_l1 gas_used_for_l2 request_id

# Arbitrum block header fields
triodion blocks --rpc https://arb1.arbitrum.io/rpc --blocks -2:latest \
    --requests-per-second 5 \
    --include-columns l1_block_number send_root send_count
```

Use `triodion help transactions` and `triodion help blocks` for the full list of available columns.

Every column added for these chain families is opt-in. The default columns are unchanged, so the output schema of an existing pipeline is byte-identical after upgrading, apart from the two columns listed under [Breaking Changes](#breaking-changes).

#### Batch size

Public L2 endpoints cap the size of a JSON-RPC batch. OP Mainnet answers `413` above ten calls and Base answers `-32014 "maximum 10 calls in 1 batch"`. `triodion` splits a batch and retries when a provider rejects it for size. Ratelimits and authentication failures are excluded from that rule, because shrinking a batch sends more requests to a node that just asked for fewer.

## Beacon Chain: Blobs and Execution Requests

The execution layer only ever sees a blob's `blob_versioned_hashes`. The blob itself lives on the consensus layer. The `blobs` dataset reads blob sidecars from a beacon node and emits one row per blob, joined back to the L1 transaction that paid for it by matching the sidecar's KZG-commitment hash against that transaction's `blob_versioned_hashes`. Nothing else links the two.

Two flags configure consensus-layer access:

| flag | env var | meaning |
| :- | :- | :- |
| `--beacon-rpc <URL>` | `BEACON_RPC_URL` | Consensus-layer (beacon) REST API url, for example `http://localhost:5052` |
| `--blob-archive <URL>` | `BLOB_ARCHIVE_URL` | Blob archive url, for slots the beacon node has pruned. The literal value `default` selects the public Blobscan API at `https://api.blobscan.com` |

For each url the resolution order is flag, then environment variable, then none. Either flag alone enables the beacon datasets; only with neither do they error. `--blob-archive` works on its own — an archive is keyed by execution block number and reports each blob's slot itself, so it needs no slot clock — but it is not a full substitute: an archive-only run has no `epoch` and cannot serve blob bytes, only their commitments.

Nothing about the chain's clock or blob schedule is compiled into the binary. Genesis time, seconds per slot, slots per epoch, and the blob schedule are read from `/eth/v1/beacon/genesis` and `/eth/v1/config/spec` when the beacon source connects. Mainnet's maximum blobs per block has moved three times since Cancun (6, then 9, then 15, then 21) and `SECONDS_PER_SLOT` differs by network, so a constant compiled into the binary is already wrong for some chain at some height. With neither `--beacon-rpc` nor `--blob-archive` the beacon datasets return an error rather than assume mainnet's clock. An archive-only run has no slot clock at all: `epoch` is null, and `slot` is whatever the archive reports.

Slot derivation is exact, not approximate: `(timestamp - genesis_time) / seconds_per_slot`. Mainnet block 20,000,000 gives slot 9,204,782, which a blob archive independently agrees with. A pre-Merge timestamp gives null, not slot 0.

#### Blob sidecars are pruned after about 18 days

A beacon node is only required to serve blob sidecars for `MIN_EPOCHS_FOR_BLOB_SIDECARS_REQUESTS` epochs, which is 4096 epochs, or about 18 days. Older sidecars are not slow to fetch. They are gone. A public Lighthouse node asked for slot 9,204,782 (execution block 20,000,000, June 2024) answers `403 Forbidden`, not an empty list, and at that layer a 403 is indistinguishable from a genuine authentication failure.

That is why the archive fallback exists, and why a beacon failure is only swallowed when an archive can answer instead. With no archive configured, the failure propagates. "The node refused" must not be recorded as "this block posted no blobs".

Every row records which source answered, in the `blob_source` column: `beacon_node` or `archive`. The two sources genuinely differ in what they carry. Only a beacon node serves the blob bytes and the proposer index. Only the archive attributes a blob to a rollup and reports the used size before zero padding. A value the answering source did not state stays null rather than taking a default.

#### Collecting blobs

The first example needs `--beacon-rpc`, because only a beacon node serves recent sidecars. The second needs an archive, because its slot was pruned long ago; `--blob-archive default` alone serves it, though then `epoch` is null and `blob` is unavailable.

```bash
# a recent block, served by the beacon node alone
triodion blobs --blocks 25803013 --rpc $ETH_RPC_URL \
    --beacon-rpc http://localhost:5052

# a historical block, served by the public Blobscan archive
triodion blobs --blocks 20000000 --rpc $ETH_RPC_URL \
    --beacon-rpc http://localhost:5052 --blob-archive default \
    --include-columns blob_used_size
```

The first collects 20 blob rows. The second collects one row: `slot` 9204782, `versioned_hash` `0x017ba4bd9c166498865a3d08618e333ee84812941b5c3a356971b4a6ffffa574`, `transaction_hash` `0x0ff07f37baa7fa26bb7de3d3fc63002bf0acf3295bdab7f67c108c0d1a3bff15`, `blob_size` 131072, `blob_used_size` 1671, `rollup` `taiko`, and `blob_source` `archive`.

`epoch`, `proposer_index`, `kzg_proof`, `transaction_index`, `blob_used_size`, and `blob` (the 131,072 bytes themselves) are opt-in columns. Use `triodion help blobs` for the full schema.

#### EIP-7685 execution requests

Prague added three request types that flow from the execution layer to the consensus layer: EIP-6110 deposits, EIP-7002 withdrawal requests, and EIP-7251 consolidations. Each gets its own dataset, because the three have nothing in common except the envelope that carries them.

| dataset | EIP | one row per |
| :- | :- | :- |
| `deposit_requests` | 6110 | validator deposit |
| `withdrawal_requests` | 7002 | exit or partial withdrawal, triggered from the execution layer |
| `consolidation_requests` | 7251 | consolidation, or a `0x01` to `0x02` credential upgrade |

The execution layer commits to all of them in one header field, `requests_hash`, and serves none of them. `eth_getBlockByNumber` returns the commitment and nothing else, and a commitment cannot be turned back into what it commits to. So these three read the consensus block and require `--beacon-rpc`:

```bash
triodion deposit_requests -b 25800355 --beacon-rpc $BEACON_RPC_URL
```

There is no archive fallback and none is needed. Beacon *blocks* are not pruned the way blob sidecars are — a slot from 2022 still answers. The node-side limit is a different one: a checkpoint-synced beacon node without backfill holds nothing before its checkpoint and answers `404`, which triodion reports as an error naming that cause rather than as an empty result.

triodion reads the *blinded* beacon block, which carries the same `execution_requests` and the same execution block number without embedding the execution payload — 15 KB against 389 KB on a measured mainnet slot. Nodes that do not serve the blinded endpoint fall back to the full block automatically. The block number the consensus block reports is checked against the one asked for, so a slot derivation that ever drifted would error instead of filing one block's requests under another.

Before Prague these datasets write no rows, which is correct rather than missing: execution requests did not exist. Deposits before Prague are still available, from the deposit contract's `DepositEvent` logs.

Two encodings in these datasets mean something other than what they look like, and each has a companion column so the trap is visible in the output:

- A `withdrawal_requests` row with `amount_gwei` of `0` is a **full exit**, not an empty request. Summing that column reports zero for the largest withdrawals on the chain. `is_full_exit` sits beside it.
- A `consolidation_requests` row whose `source_pubkey` equals its `target_pubkey` is a **credential upgrade** to the compounding kind, not a merge of two validators. These were the majority of such requests after Prague. `is_credential_upgrade` sits beside it.

#### EIP-4895 withdrawals are not a beacon dataset

`withdrawals` looks like it belongs above and does not. EIP-4895 withdrawals are in the execution block body, so an ordinary RPC url is enough.

The dataset emits one row per withdrawal, where [blocks](book/datasets/blocks.md) carries only `withdrawals_count` and `withdrawals_amount_gwei`. An aggregate cannot be taken apart afterwards, so validator-level payouts were previously unrecoverable from triodion's output.

A withdrawal has no sender, no gas cost, no receipt and no transaction hash, and it never executes — which means it appears in no other dataset. An ETH-flow analysis built from `traces` and `native_transfers` alone is missing every validator payout on the chain.

#### Known limitation: missed slots

The beacon datasets are indexed by execution block, and derive the slot from the block timestamp. The derivation is exact, but a missed slot produces no execution block, so a `--blocks` run cannot see one.

## Breaking Changes

Two `transactions` columns changed shape. Neither is a default column, so a pipeline that did not name one of them explicitly is unaffected.

**`v` is now `uint64`, and was `bool`.** The old column held alloy's `Signature::v()`, which is the y-parity *bit* and not the EIP-155 `v` *scalar*. Every legacy row was therefore mislabelled, and the chain id that EIP-155 folds into `v` was unrecoverable. `v` now holds the scalar as it appears on the wire: `27` or `28` for an unprotected legacy transaction, `chain_id * 2 + 35 + parity` for a replay-protected one, and `0` or `1` for every typed transaction. The parity bit is kept beside it in the new `y_parity` column, which is what the old `v` column actually contained. Unsigned transaction types, which are the OP deposits and the Arbitrum internal transactions, report null for `r`, `s`, `v`, and `y_parity`, rather than the zeros the node sends. Storing those zeros would claim a signature exists.

**`n_rlp_bytes` is now a nullable `uint32`, and was `uint32`.** It is computed from alloy's EIP-2718 encoded length, which panics for exactly the non-Ethereum type bytes this release adds support for. The column is null for those transactions, rather than a number `triodion` cannot compute.

One behaviour changed without a change of schema: `gas_price` now prefers the receipt's `effectiveGasPrice`, then the value the transaction itself reports, and only computes a price when neither is available. The old code went straight to the EIP-1559 formula through unchecked unwraps, so a typed transaction in a block with no base fee, which describes every OP deposit, aborted the worker task.

## Code Guide
- Code is arranged into the following crates:
    - `triodion-cli`: convert textual data into triodion function calls
    - `triodion-core`: core triodion code
    - `triodion-py`: python adapter (imported as `triodion`, distributed as `triodion`)
    - `triodion-macros`: procedural macro for generating dataset definitions
- Do not use panics (including `panic!`, `todo!`, `unwrap()`, and `expect()`) except in the following circumstances: tests, build scripts, lazy static blocks, and procedural macros

## Documentation

1. [triodion help](#triodion-help)
2. [triodion syntax](#triodion-syntax)
3. [triodion datasets](#triodion-datasets)

#### triodion help

(output of `triodion help`)

```
triodion extracts blockchain data to parquet, csv, or json

Usage: triodion [OPTIONS] [DATATYPE]...

Arguments:
  [DATATYPE]...  datatype(s) to collect, use triodion datasets to see all available

Options:
      --remember    Remember current command for future use
  -v, --verbose     Extra verbosity
      --no-verbose  Run quietly without printing information to stdout
  -h, --help        Print help (see more with '--help')
  -V, --version     Print version

Content Options:
  -b, --blocks <BLOCKS>...            Block numbers, see syntax below
      --timestamps [<TIMESTAMPS>...]  Timestamps in unix, see syntax below
  -t, --txs <TXS>...                  Transaction hashes, see syntax below
  -a, --align                         Align chunk boundaries to regular intervals,
                                      e.g. (1000 2000 3000), not (1106 2106 3106)
      --reorg-buffer <N_BLOCKS>       Reorg buffer, save blocks only when this old,
                                      can be a number of blocks [default: 0]
  -i, --include-columns [<COLS>...]   Columns to include alongside the defaults,
                                      use `all` to include all available columns
  -e, --exclude-columns [<COLS>...]   Columns to exclude from the defaults
      --columns [<COLS>...]           Columns to use instead of the defaults,
                                      use `all` to use all available columns
      --u256-types <U256_TYPES>...    Set output datatype(s) of U256 integers
                                      [default: binary, string, f64]
      --hex                           Use hex string encoding for binary columns
  -s, --sort [<SORT>...]              Columns(s) to sort by, `none` for unordered
      --exclude-failed                Exclude items from failed transactions

Source Options:
  -r, --rpc <RPC>                    RPC url [default: 1. MESC 2. ETH_RPC_URL]
      --l1-rpc <URL>                 L1 (settlement) RPC url for L2 datasets that read L1-side events
      --beacon-rpc <URL>             Consensus-layer (beacon) REST API url, e.g. http://localhost:5052
      --blob-archive <URL>           Blob archive url, for slots the beacon node has pruned
      --network-name <NETWORK_NAME>  Network name [default: name of eth_getChainId]

Acquisition Options:
  -l, --requests-per-second <limit>   Ratelimit on requests per second
      --max-retries <R>               Max retries for provider errors [default: 5]
      --initial-backoff <B>           Initial retry backoff time (ms) [default: 500]
      --compute-units-per-second <U>  The number of compute units per second for this provider [default: 50]
      --max-concurrent-requests <M>   Global number of concurrent requests
      --max-concurrent-chunks <M>     Number of chunks processed concurrently
      --chunk-order <CHUNK_ORDER>     Chunk collection order (normal, reverse, random)
  -d, --dry                           Dry run, collect no data

Output Options:
  -c, --chunk-size <CHUNK_SIZE>      Number of blocks per file [default: 1000]
      --n-chunks <N_CHUNKS>          Number of files (alternative to --chunk-size)
      --partition-by <PARTITION_BY>  Dimensions to partition by
  -o, --output-dir <OUTPUT_DIR>      Directory for output files [default: .]
      --subdirs <SUBDIRS>...         Subdirectories for output files
                                     can be `datatype`, `network`, or custom string
      --label <LABEL>                Label to add to each filename
      --overwrite                    Overwrite existing files instead of skipping
      --csv                          Save as csv instead of parquet
      --json                         Save as json instead of parquet
      --row-group-size <GROUP_SIZE>  Number of rows per row group in parquet file
      --n-row-groups <N_ROW_GROUPS>  Number of rows groups in parquet file
      --no-stats                     Do not write statistics to parquet files
      --compression <NAME [#]>...    Compression algorithm and level [default: lz4]
      --report-dir <REPORT_DIR>      Directory to save summary report
                                     [default: {output_dir}/.triodion/reports]
      --no-report                    Avoid saving a summary report

Dataset-specific Options:
      --address <ADDRESS>...         Address(es)
      --to-address <address>...      To Address(es)
      --from-address <address>...    From Address(es)
      --call-data <CALL_DATA>...     Call data(s) to use for eth_calls
      --function <FUNCTION>...       Function(s) to use for eth_calls
      --inputs <INPUTS>...           Input(s) to use for eth_calls
      --slot <SLOT>...               Slot(s)
      --contract <CONTRACT>...       Contract address(es)
      --topic0 <TOPIC0>...           Topic0(s) [aliases: event]
      --topic1 <TOPIC1>...           Topic1(s)
      --topic2 <TOPIC2>...           Topic2(s)
      --topic3 <TOPIC3>...           Topic3(s)
      --event-signature <SIG>...     Event signature for log decoding
      --inner-request-size <BLOCKS>  Blocks per request (eth_getLogs) [default: 1]
      --js-tracer <tracer>           Event signature for log decoding
      --no-multicall                 Disable Multicall3 batching for eth_calls / erc20_balances
      --multicall-batch-size <N>     Cap on inner eth_calls per Multicall3 batch (0 = the dataset's own default) [default: 0]
      --multicall-require-success    Mark the whole batch as failed if any inner call reverts (default: per-call failures return null)

Optional Subcommands:
      triodion help                      display help message
      triodion help syntax               display block + tx specification syntax
      triodion help datasets             display list of all datasets
      triodion help <DATASET(S)>         display info about a dataset
```

#### triodion syntax

(output of `triodion help syntax`)

```
Block specification syntax
- can use numbers                    --blocks 5000 6000 7000
- can use ranges                     --blocks 12M:13M 15M:16M
- can use a parquet file             --blocks ./path/to/file.parquet[:COLUMN_NAME]
- can use multiple parquet files     --blocks ./path/to/files/*.parquet[:COLUMN_NAME]
- numbers can contain { _ . K M B }  5_000 5K 15M 15.5M
- omitting range end means latest    15.5M: == 15.5M:latest
- omitting range start means 0       :700 == 0:700
- minus on start means minus end     -1000:7000 == 6000:7000
- plus sign on end means plus start  15M:+1000 == 15M:15.001K
- can use every nth value            2000:5000:1000 == 2000 3000 4000
- can use n values total             100:200/5 == 100 124 149 174 199

Transaction specification syntax
- can use transaction hashes         --txs TX_HASH1 TX_HASH2 TX_HASH3
- can use a parquet file             --txs ./path/to/file.parquet[:COLUMN_NAME]
                                     (default column name is transaction_hash)
- can use multiple parquet files     --txs ./path/to/ethereum__logs*.parquet
```

#### triodion datasets

(output of `triodion help datasets`)

```
triodion datasets
─────────────────
- access_lists
- address_appearances
- authorizations
- balance_diffs
- balance_reads
- balances
- blobs
- blocks
- code_diffs
- code_reads
- codes
- consolidation_requests
- contracts
- deposit_requests
- erc20_balances
- erc20_metadata
- erc20_supplies
- erc20_transfers
- erc20_approvals
- erc20_wrapper_events (aliases = wrapper_events, weth_events)
- erc721_metadata
- erc721_transfers
- eth_calls
- four_byte_counts (alias = 4byte_counts)
- geth_calls
- geth_code_diffs
- geth_balance_diffs
- geth_storage_diffs
- geth_nonce_diffs
- geth_opcodes
- javascript_traces (alias = js_traces)
- logs (alias = events)
- native_transfers
- nonce_diffs
- nonce_reads
- nonces
- slots (alias = storages)
- storage_diffs (alias = slot_diffs)
- storage_reads (alias = slot_reads)
- traces
- trace_calls
- transactions (alias = txs)
- vm_traces (alias = opcode_traces)
- withdrawal_requests
- withdrawals

dataset group names
───────────────────
- blocks_and_transactions: blocks, transactions
- call_trace_derivatives: contracts, native_transfers, traces
- geth_state_diffs: geth_balance_diffs, geth_code_diffs, geth_nonce_diffs, geth_storage_diffs
- log_events: logs, erc20_transfers, erc20_approvals, erc721_transfers, erc20_wrapper_events
- state_diffs: balance_diffs, code_diffs, nonce_diffs, storage_diffs
- state_reads: balance_reads, code_reads, nonce_reads, storage_reads

use triodion help <DATASET> to print info about a specific dataset
```
