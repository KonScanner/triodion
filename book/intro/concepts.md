# Concepts

## Datasets

A dataset is one table with one schema. `blocks`, `transactions`, `logs` and
`traces` are datasets. Each one knows which RPC calls it needs, which columns
it can produce, and how its rows are ordered. `triodion datasets` lists them
all, and `triodion help <DATASET>` prints one dataset's schema and parameters.

Several datasets can be collected in a single command. Some names are groups
rather than single tables: `blocks_and_transactions` expands to `blocks` and
`transactions`, `state_diffs` to the four diff datasets, and so on. Groups
exist because the datasets inside them come from the same RPC response, so
collecting them together costs one round trip rather than several.

Datasets differ in what they need from the node. See
[Data Sources](./data_sources.md#node-capabilities).

## Columns

Every dataset has more columns than it emits. The schema is split in two:

- **default columns**, which appear unless you say otherwise
- **other available columns**, which appear only when asked for

Three options move columns across that line:

| Option | Effect |
| :- | :- |
| `-i`, `--include-columns` | Defaults, plus the named columns. `all` includes everything |
| `-e`, `--exclude-columns` | Defaults, minus the named columns |
| `--columns` | Exactly the named columns. `all` uses everything |

All three take space-separated names, not a comma-separated list.

The split is deliberate. The defaults are what most queries want, and the rest
are opt-in so that a schema does not grow under an existing pipeline. Adding a
column to the "other available" list never changes anyone's output.

Cryptographic material that identifies nothing is off by default for this
reason: a block's `state_root`, or a transaction's `r`, `s` and `v`.

Some default columns are worth dropping on the other side. `chain_id` is in
every dataset's default set, and it holds the same value in every row of every
file — the network is already in the output filename and the run report. It is
kept on by default because removing it would change the output schema of every
existing pipeline. Drop it when you do not want it, and the flag applies to
every dataset in the run:

```bash
triodion blocks transactions -b 20M:+100 -e chain_id
```

A `u256` column is emitted once per requested representation. With the default
`--u256-types binary string f64`, a `value` column becomes `value_binary`,
`value_string` and `value_f64`. `f32`, `u32`, `u64` and `d128` are also
available. There is no single lossless integer type in parquet wide enough for
a `u256`, so the choice is left to the caller.

Use `--dry` to print the schema a command would produce without collecting
anything.

## Chunks

A block range is not collected as one unit. It is divided into chunks, and each
chunk becomes one output file. `--chunk-size` sets the blocks per file, and
defaults to `1000`. `--n-chunks` sets the number of files instead.

`--align` rounds chunk boundaries to the chunk size, so a range starting at
block 1,106 produces files starting at 2,000 rather than at 1,106. Aligned
files from separate runs line up with each other.

Chunks are the unit of resumption. A file that already exists is skipped, so
re-running a command fills in only what is missing. `--overwrite` disables
that.

`--reorg-buffer` holds back the chain tip: a block is written only once it is
that many blocks old. At the tip a block can still be reorganised out of
existence, and a file written from it would be wrong with no later run to
correct it.

## Partitions

Chunking splits by block. Partitioning splits by anything else the query ranges
over. `--partition-by` takes one or more dimensions:

`block`, `transaction`, `call_data`, `address`, `contract`, `from_address`,
`to_address`, `slot`, `topic0`, `topic1`, `topic2`, `topic3`

A query for three contracts across a million blocks can be written as one file
per contract per chunk, rather than one file holding all three. Which
dimensions are available depends on the dataset, since a dataset only ranges
over the parameters it accepts.

## Block indexing and transaction indexing

Most datasets can be driven two ways:

- by block, with `-b`/`--blocks`
- by transaction, with `-t`/`--txs`

`triodion help <DATASET>` states which of the two a dataset supports, as
"can collect by block or by transaction" or "can collect by block and not by
transaction". The distinction matters because it decides which RPC call is
made: a block-indexed run asks the node about a block, a transaction-indexed
run asks about one transaction hash.

`--timestamps` is a third way to name blocks. Each timestamp is resolved by
binary search to the closest block at or before it, and the run proceeds by
block from there.

Block ranges have their own syntax, including open ends, steps and parquet
files as input. Run `triodion help syntax`.

## Chain families

triodion targets three EVM chain families:

| Family | `chain_family` value | Examples |
| :- | :- | :- |
| Ethereum | `ethereum` | Ethereum mainnet, and most L1s |
| OP stack | `op_stack` | OP Mainnet, Base |
| Arbitrum stack | `arbitrum` | Arbitrum One, Nova |

The families differ in their EIP-2718 transaction type bytes. Ethereum defines
`0x00` to `0x04`. The OP stack adds the deposit transaction at `0x7e`, and the
Arbitrum stack adds `0x64`–`0x66`, `0x68`–`0x6a` and `0x78`; `0x67` is
deliberately unassigned in the Arbitrum fork, so triodion classifies it as
`unknown` rather than claiming it for a family whose field names would not
apply. Those type bytes are not
decorative: every OP-stack block opens with a deposit transaction, and every
Arbitrum block opens with an ArbOS internal transaction.

triodion therefore decodes blocks as alloy's `AnyNetwork` rather than its
`Ethereum` network, which models Ethereum's type bytes only. Each transaction
row can carry its family in the `chain_family` column.

Family-specific columns exist for both stacks — OP deposit fields and L1-fee
receipt fields, Arbitrum L1/L2 gas accounting and retryable-ticket fields — and
all of them are opt-in. A run against mainnet sees no change.

Some concepts do not survive the crossing. An Arbitrum `gas_used` is not
comparable to a mainnet one, and an unsigned transaction — an OP deposit, an
Arbitrum internal transaction — has no signature, so `r`, `s`, `v` and
`y_parity` are null rather than zero.

## Slots and blocks

Consensus-layer data is indexed by slot, not by block. triodion reaches it from
the execution side: it derives the slot from the block timestamp, as
`(timestamp - genesis_time) / seconds_per_slot`, using the genesis time and
slot length read from the beacon node at connect time.

The derivation is exact. Mainnet block 20,000,000 gives slot 9,204,782, and a
pre-Merge timestamp gives null rather than slot 0.

It has one limitation. A missed slot produces no execution block, so a run
driven by `--blocks` cannot see one. Every slot that a block-indexed run
reports is a slot that was actually filled.

Five datasets sit on this boundary, and they do not all sit on the same side
of it:

| dataset | source | needs |
| :- | :- | :- |
| [blobs](../datasets/blobs.md) | consensus | `--beacon-rpc` or `--blob-archive` |
| [deposit_requests](../datasets/deposit_requests.md) | consensus | `--beacon-rpc` |
| [withdrawal_requests](../datasets/withdrawal_requests.md) | consensus | `--beacon-rpc` |
| [consolidation_requests](../datasets/consolidation_requests.md) | consensus | `--beacon-rpc` |
| [withdrawals](../datasets/withdrawals.md) | execution | an RPC url |

The last one is the surprise. EIP-4895 withdrawals are credited by the
consensus layer but delivered in the execution block body, so reading them
needs no beacon node at all.

The three request datasets exist because EIP-7685 commits to execution requests
in the header, as `requests_hash`, and puts the requests themselves nowhere an
execution node will serve them. A commitment cannot be turned back into what it
commits to, so the consensus block is the only source.

Retention differs between the two kinds. Blob sidecars are pruned after about
eighteen days. Beacon *blocks* are not, so the request datasets need no archive
and have none.

See [Data Sources](./data_sources.md#the-consensus-layer) for the beacon
endpoint, the blob archive, and the retention window behind them.
