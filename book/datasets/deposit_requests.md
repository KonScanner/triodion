# deposit_requests

One row per EIP-6110 deposit request. Prague onward.

- can collect by block and not by transaction
- required parameters: [none]
- optional parameters: [none]
- dataset aliases: [none]

Needs `--beacon-rpc`. See [Execution requests](#why-this-needs-a-beacon-node).

## Schema

```
schema for deposit_requests
───────────────────────────
- block_number: uint32
- timestamp: uint32
- slot: uint64
- request_index: uint32
- pubkey: binary
- withdrawal_credentials_type: uint32
- withdrawal_address: binary
- amount_gwei: uint64
- deposit_index: uint64
- chain_id: uint64

sorting deposit_requests by: block_number, request_index

other available columns: epoch, proposer_index, withdrawal_credentials, signature
```

Run `triodion help deposit_requests` to print this from the binary.

## Why this needs a beacon node

EIP-7685 puts a single `requests_hash` in the block header, committing to all
of a block's execution requests. It does not put the requests in the block.
`eth_getBlockByNumber` returns the commitment and nothing else, and a
commitment cannot be turned back into what it commits to.

The consensus block is where the decoded requests live, so these three datasets
read the beacon REST API. Unlike [blobs](./blobs.md) there is no archive
fallback and none is needed: beacon *blocks* are not pruned the way blob
sidecars are. A slot from 2022 still answers.

The one node-side limit is different: a checkpoint-synced beacon node without
backfill holds nothing before its checkpoint, and answers `404`. triodion
reports that as an error naming the cause rather than as an empty result.

triodion reads the *blinded* beacon block, which carries the same requests and
the same execution block number without embedding the execution payload — about
15 KB against 389 KB on a measured mainnet slot. Nodes that do not serve the
blinded endpoint fall back to the full block automatically.

## EIP-6110 changed the route, not the deposit

Before Prague, the consensus layer discovered deposits by voting on execution
state. From Prague, the block carries them directly. Both forms originate in
the same call to the deposit contract.

So for history before Prague this dataset writes no rows, and the deposits are
not missing — they are in the deposit contract's `DepositEvent` logs, which
`logs` can read for the whole history of the chain:

```bash
triodion logs -b <RANGE> --address 0x00000000219ab540356cBB839Cbe05303d7705Fa
```

## Withdrawal credentials

`withdrawal_credentials` is 32 bytes whose first byte states the kind:

| First byte | Meaning | `withdrawal_address` |
| :- | :- | :- |
| `0x00` | BLS credentials | null |
| `0x01` | execution address | the last 20 bytes |
| `0x02` | compounding (EIP-7251) | the last 20 bytes |

`withdrawal_credentials_type` is that first byte, and `withdrawal_address` is
the address for kinds `0x01` and `0x02` only.

For `0x00` the remaining bytes are a hashed BLS key. Reading 20 of them as an
address would produce a well-formed address belonging to nobody, so
`withdrawal_address` is null there. It is not "unknown" — those bytes are
simply not an address.

`withdrawal_address` is the join key to [withdrawals](./withdrawals.md), which
is where money is eventually paid to it.

## Identity

`deposit_index` is the deposit contract's own global, monotonic sequence
number. It identifies a deposit across the whole chain and is the column to
join on. `request_index` is only the position within this one block.
