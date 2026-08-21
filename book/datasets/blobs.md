# blobs

One row per EIP-4844 blob, joined to the L1 transaction that paid for it.

- can collect by block and not by transaction
- required parameters: [none]
- optional parameters: [none]
- dataset aliases: [none]

This is the only triodion dataset that reads the consensus layer. It needs
`--beacon-rpc`, and for anything older than about eighteen days it also needs
`--blob-archive`.

## Why a blob is not on the execution layer

A blob is 4096 field elements, 131,072 bytes, carried alongside a block rather
than inside it. The execution layer never sees the bytes. A type `0x03`
transaction commits only to a list of versioned hashes, and a versioned hash is
`0x01` followed by `sha256(kzg_commitment)[1..]`. The blob itself is published
on the beacon chain as a sidecar.

So there is no RPC method on an execution client that returns a blob. Reading
one means asking a consensus-layer node, over a different protocol: a REST API
rather than JSON-RPC, no batching, decimal-string integers rather than hex
quantities, and slots rather than block numbers.

## Schema

```
schema for blobs
────────────────
- block_number: uint32
- timestamp: uint32
- slot: uint64
- blob_index: uint32
- versioned_hash: binary
- kzg_commitment: binary
- transaction_hash: binary
- from_address: binary
- to_address: binary
- blob_size: uint64
- rollup: string
- blob_source: string
- chain_id: uint64

sorting blobs by: block_number, blob_index

other available columns: epoch, proposer_index, kzg_proof, transaction_index,
blob_used_size, blob
```

Run `triodion help blobs` to print this from the binary.

## Joining to `transactions`

`versioned_hash` is the join key, and it is the only one. A blob sidecar does
not name the transaction that posted it. The link exists solely in the
execution-layer transaction's `blob_versioned_hashes`.

triodion builds that link while collecting: it fetches the block with full
transaction bodies, maps every versioned hash in the block to the transaction
that carries it, and matches each sidecar's KZG-commitment hash against that
map. `transaction_hash`, `transaction_index`, `from_address` and `to_address`
are the result of that match.

Against the [transactions](./transactions.md) dataset, the same key works from
the other side: `blob_versioned_hashes` there is the concatenation of a
transaction's hashes, 32 bytes each, in commitment order.

`transaction_hash` is null when a blob's hash appeared in no transaction in the
block. That should not happen. It is reported rather than hidden.

## Flags

| flag | environment variable | meaning |
| --- | --- | --- |
| `--beacon-rpc <URL>` | `BEACON_RPC_URL` | consensus-layer REST API, e.g. `http://localhost:5052` |
| `--blob-archive <URL>` | `BLOB_ARCHIVE_URL` | archive for slots the beacon node has pruned |

`--blob-archive default` resolves to the public Blobscan API at
`https://api.blobscan.com`.

`--beacon-rpc` is always required, including on the archive path: the consensus
source is only built when a beacon URL is given, and the slot clock is read
from that node.

## The retention window

A beacon node is only required to serve blob sidecars for
`MIN_EPOCHS_FOR_BLOB_SIDECARS_REQUESTS` epochs. That is 4096 epochs, about 18
days. After that the node drops them. Older blobs are not slow to fetch. They
are gone, and no beacon node will return them.

This is why the dataset takes two endpoints instead of one, and why it records
which one answered instead of presenting them as interchangeable.

### A pruned slot answers 403, not 404

A public Lighthouse node asked for slot 9,204,782, which is execution block
20,000,000 from June 2024, answers `403 Forbidden`. It does not answer with an
empty list, and it does not answer 404.

At the transport layer a 403 is indistinguishable from a genuine authorisation
failure, so triodion treats it as an error rather than as "nothing here". A 404
is treated as "nothing here", because some clients report an empty slot that
way.

What happens to that error depends on whether an archive is configured:

- with an archive, the error is swallowed and the archive is asked instead. The `blob_source`
  column already records that the node did not answer, so the error is redundant.
- with no archive, the error propagates and the run fails. This is deliberate. "The node refused"
  must not be silently reported as "this block posted no blobs".

## `blob_source`

Every row records who answered, as `beacon_node` or `archive`.

A blob-free block and a block nobody would answer for both produce zero rows,
so a column of empty results cannot distinguish them. `blob_source` says who
was asked whenever rows do appear.

The two sources genuinely differ, and neither gap is filled with a default:

| column | beacon node | archive |
| --- | --- | --- |
| `blob` | the 131,072 bytes | null |
| `proposer_index` | present | null |
| `blob_used_size` | null | present |
| `rollup` | null | present when attributable |

Only the beacon node serves the blob bytes. Only the archive can attribute a
blob to a rollup.

## `blob`

The `blob` column holds the blob itself: 131,072 bytes, or 128 KiB, per row.

It is opt-in for that reason. A block at the current mainnet maximum of 21
blobs would add roughly 2.7 MB of output for that one block. Ask for it only
when you need the payload:

```bash
triodion blobs -b 25803013 -i blob --beacon-rpc http://localhost:5052
```

`blob_size` is 131,072 for a well-formed blob. `blob_used_size` is the bytes
used before zero padding, and the gap between the two is padding the poster
paid for.

## Slots are derived, not guessed

Nothing about the chain's clock is compiled into the binary. Genesis time,
seconds per slot, slots per epoch and the blob schedule are all read from
`/eth/v1/beacon/genesis` and `/eth/v1/config/spec` when the beacon connection
is made.

That is not caution for its own sake. Mainnet's blob maximum has moved three
times since Cancun, 6 to 9 to 15 to 21, and `SECONDS_PER_SLOT` differs between
networks. Any constant baked into the binary is already wrong for some chain at
some height. Without `--beacon-rpc` the beacon datasets error rather than
assume mainnet's numbers.

Slot derivation is then exact, not approximate:

```
slot = (block_timestamp - genesis_time) / seconds_per_slot
```

Slots are a fixed-duration clock started at genesis, and an execution block's
timestamp is its slot's start time. Mainnet block 20,000,000 gives slot
9,204,782, which a blob archive independently agrees with.

A timestamp before genesis, which on mainnet means a pre-Merge block, gives
null rather than slot 0. Rounding one to zero would invent a join key.

The `epoch` column is derived from `slot` using 32 slots per epoch. Every chain
in the wild uses 32, but `slot` is the primary key here and `epoch` is a
convenience.

## Known limitation: missed slots

The beacon datasets are indexed by execution block, and derive the slot from
the block timestamp. That derivation is exact, but it is not reversible: a
missed slot produces no execution block.

A `--blocks` run therefore cannot see a missed slot. If your question is about
slots that were skipped, this dataset cannot answer it.

## Worked examples

Both were run end to end against live endpoints.

### A recent block, beacon node only

```bash
triodion blobs -b 25803013 --beacon-rpc <BEACON_URL>
```

20 rows. The block is inside the retention window, so the beacon node serves
it and every row reports `blob_source` = `beacon_node`.

### A historical block, through the archive

```bash
triodion blobs -b 20000000 --beacon-rpc <BEACON_URL> --blob-archive default \
    -i blob_used_size
```

1 row:

```
slot              9204782
versioned_hash    0x017ba4bd9c166498865a3d08618e333ee84812941b5c3a356971b4a6ffffa574
transaction_hash  0x0ff07f37baa7fa26bb7de3d3fc63002bf0acf3295bdab7f67c108c0d1a3bff15
blob_size         131072
blob_used_size    1671
rollup            taiko
blob_source       archive
```

June 2024 is far outside the retention window, so `blob_source` is `archive`.
`blob_used_size` shows what that blob actually carried: 1,671 bytes of the
131,072 the poster paid for.

Without `--blob-archive`, this run fails instead of returning zero rows.
