# Data Sources

triodion reads from as many as three networks. Only the first one is required.

| Layer | Flag | Env var | Purpose | Datasets that need it |
| :- | :- | :- | :- | :- |
| Execution | `--rpc <URL>` | `ETH_RPC_URL` (MESC is tried first) | The chain itself: blocks, transactions, logs, traces, state | every dataset |
| L1 settlement | `--l1-rpc <URL>` | `L1_RPC_URL` | A second execution node, for L2 runs that must read L1-side data | none in this release, see below |
| Consensus | `--beacon-rpc <URL>` | `BEACON_RPC_URL` | The beacon chain REST API: slot clock and blob sidecars | `blobs` |
| Blob archive | `--blob-archive <URL>` | `BLOB_ARCHIVE_URL` | Blobs the beacon node has already pruned | `blobs`, for blocks older than about 18 days |

## The execution node

Every run needs one execution-layer JSON-RPC endpoint. triodion picks it in this
order:

1. MESC, if MESC is enabled. Note that a `--rpc` value is then treated as a MESC *query* rather than as a url, and only a MESC miss falls through to the next step
2. the `--rpc` value, used literally
3. the `ETH_RPC_URL` environment variable

The network name comes from the chain id that the node reports. Use
`--network-name` to override it.

### Node capabilities

A dataset is only as available as the RPC namespace it calls. There are four
tiers, and a node that answers one tier can refuse the next.

**Standard JSON-RPC.** `blocks`, `transactions`, `logs` and everything derived
from logs (`erc20_transfers`, `erc20_approvals`, `erc721_transfers`,
`erc20_wrapper_events`) need only `eth_getBlockByNumber`,
`eth_getBlockReceipts` and `eth_getLogs`. Any node serves these.

**The `trace_*` namespace.** `traces`, `trace_calls`, `vm_traces`, `contracts`,
`native_transfers`, `address_appearances` and the `state_diffs` group
(`balance_diffs`, `code_diffs`, `nonce_diffs`, `storage_diffs`) call
`trace_block`, `trace_replayBlockTransactions` or `trace_call`. This namespace
comes from the OpenEthereum lineage. Erigon, Reth and Nethermind serve it. A
default geth node does not.

**The `debug_*` namespace.** The `geth_` datasets (`geth_calls`,
`geth_opcodes`, `geth_balance_diffs`, `geth_code_diffs`, `geth_nonce_diffs`,
`geth_storage_diffs`), `javascript_traces`, `four_byte_counts` and the
`state_reads` group (`balance_reads`, `code_reads`, `nonce_reads`,
`storage_reads`) call `debug_traceBlockByNumber` or
`debug_traceTransaction` with the call tracer, the prestate tracer or a custom
JavaScript tracer. Many hosted providers disable `debug_*`, or price it apart
from the rest.

**Archive state.** `balances`, `codes`, `nonces` and `slots` read state at a
past block through `eth_getBalance`, `eth_getCode`, `eth_getTransactionCount`
and `eth_getStorageAt`. `eth_calls`, `erc20_balances`, `erc20_supplies`,
`erc20_metadata` and `erc721_metadata` send `eth_call` at a past block. A
pruned node holds only the most recent state and answers "missing trie node"
for anything older. These datasets need an archive node.

Use `--dry` to see the schema and the planned work before a run spends any
requests.

### Chain families

The execution datasets work on Ethereum, on the OP stack (OP Mainnet, Base) and
on the Arbitrum stack (Arbitrum One, Nova). These chains define EIP-2718
transaction type bytes that Ethereum does not, so triodion decodes blocks with
alloy's `AnyNetwork` rather than its `Ethereum` network. The stack-specific
columns are available but off by default. See
[Concepts](./concepts.md#chain-families).

## The L1 settlement node

`--l1-rpc` opens a second execution provider, for L2 runs that must read
something from the settlement chain. The url can also come from `L1_RPC_URL`.
triodion connects to it at start-up and reads its chain id, so a bad url fails
the run immediately rather than halfway through.

Both providers share one concurrency limit and one rate limiter. A request to
L1 therefore counts against the same budget as a request to L2.

No dataset in this release reads the L1 provider yet. Setting the flag today
costs one extra chain-id call and nothing else.

## The consensus layer

The execution layer never carries a blob. A type-3 transaction records only the
`blob_versioned_hashes` it commits to; the 131,072 bytes behind each hash live
on the beacon chain. To collect blobs, triodion needs a consensus-layer
endpoint as well as an execution one.

```bash
triodion blobs -b 25.8M --rpc $ETH_RPC_URL --beacon-rpc http://localhost:5052
```

A beacon node is a REST API, not JSON-RPC. It answers on paths such as
`/eth/v1/beacon/blob_sidecars/{slot}`, and it writes numbers as decimal
strings, not hex quantities.

### Nothing about the chain is compiled in

Genesis time, seconds per slot, slots per epoch and the blob schedule are read
from `/eth/v1/beacon/genesis` and `/eth/v1/config/spec` when triodion connects.
They are not constants in the binary, because they move: mainnet's blob maximum
per block has changed three times since Cancun (6, then 9, then 15, then 21),
and `SECONDS_PER_SLOT` differs between networks. Without either flag the
beacon datasets return an error rather than assume mainnet's clock.

Slots are derived exactly, as `(timestamp - genesis_time) / seconds_per_slot`.
Mainnet block 20,000,000 gives slot 9,204,782, which a blob archive
independently agrees with. A pre-Merge timestamp gives null, not slot 0.

### The retention window

A beacon node keeps blob sidecars for
`MIN_EPOCHS_FOR_BLOB_SIDECARS_REQUESTS` epochs. That is 4096 epochs, about 18
days. After that it drops them, and it is not obliged to be polite about the
request. A public Lighthouse node asked for slot 9,204,782 (execution block
20,000,000, June 2024) answers `403 Forbidden`, not an empty list.

This is why a beacon failure is only swallowed when an archive can answer
instead. With no archive configured the error propagates, because "the node
refused" must never be recorded as "this block posted no blobs".

Pass `--blob-archive` to reach older blobs. The literal value `default` selects
the public Blobscan API at `https://api.blobscan.com`:

```bash
triodion blobs -b 20M --beacon-rpc http://localhost:5052 --blob-archive default
```

`--blob-archive` is a fallback inside the consensus-layer client, and it is
also usable on its own: an archive is keyed by execution block number and
reports each blob's slot itself, so it needs no slot clock. Only a beacon node
serves the blob bytes, so an archive-only run leaves `blob` and `epoch` null.
With neither flag the beacon datasets build no consensus-layer access
at all.

Every blob row records which side answered, in the `blob_source` column:
`beacon_node` or `archive`. Only a beacon node serves the bytes themselves, so
archive rows leave the `blob` column null.

### A limitation worth stating

Beacon datasets are indexed by execution block, and the slot is derived from
that block's timestamp. The derivation is exact, but a missed slot produces no
execution block. A run driven by `--blocks` therefore cannot see one.
