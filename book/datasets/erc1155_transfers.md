# erc1155_transfers

One row per token id moved, over both ERC-1155 transfer events.

```
erc1155_transfers
─────────────────
- can collect by block or by transaction
- required parameters: [none]
- optional parameters: address, topic1, from_address, to_address
- dataset aliases: [none]
- parameter aliases: 
    - contracts -> addresses


schema for erc1155_transfers
────────────────────────────
- block_number: uint32
- transaction_index: uint32
- log_index: uint32
- token_id_index: uint32
- transaction_hash: binary
- erc1155: binary
- operator: binary
- from_address: binary
- to_address: binary
- token_id_binary: binary
- token_id_string: string
- token_id_f64: float64
- value_binary: binary
- value_string: string
- value_f64: float64
- is_batch: bool
- is_mint: bool
- is_burn: bool
- chain_id: uint64

sorting erc1155_transfers by: block_number, log_index, token_id_index

other available columns: block_hash
```

Run `triodion help erc1155_transfers` to print this from the binary.

## One table, two events

ERC-1155 defines two transfer events:

```solidity
TransferSingle(address indexed operator, address indexed from, address indexed to,
               uint256 id, uint256 value)
TransferBatch (address indexed operator, address indexed from, address indexed to,
               uint256[] ids, uint256[] values)
```

Both describe the same thing — an amount of one token id moving from one party
to another — so both land in this table. `is_batch` says which event a row came
from.

Both carry a signature plus three indexed arguments, so the topic count cannot
tell them apart. topic0 is the only discriminator.

## A batch becomes rows, not an array

`TransferBatch` carries two parallel arrays, `ids` and `values`. Storing them
whole, or storing only their length, would put an aggregate in a cell that
cannot be taken apart again: which token moved, and how much of it, would be
gone.

A batch of five ids therefore becomes five rows, and a `TransferSingle` becomes
one row of exactly that shape. Every column outside `is_batch` reads the same
either way, so a query never has to branch on which event it was.

`token_id_index` is the position of a token id within its own log's `ids` list,
and `0` on a `TransferSingle`. Batch order is emitted order and carries meaning,
so it is kept rather than reconstructed. `(log_index, token_id_index)` is the
unique key within a block.

The name is not `transfer_index`: that word already counts within a block in
[native_transfers](./native_transfers.md), and one word keeps one meaning
across datasets.

## Sort order

The default sort is `block_number, log_index, token_id_index`, one column
longer than the log-dataset default.

Stopping at `log_index` leaves every row of a batch tied, and polars does not
promise to keep tied rows in input order. Sorting on the ordinal too makes the
order total, so emitted batch order survives the write and is the same on every
run.

## operator, and the two zero-address flags

`operator` is the `msg.sender` that performed the transfer. It need not be
`from`: an approved operator moves someone else's tokens under their own name.
See [approvals_for_all](./approvals_for_all.md) for where that permission comes
from.

`is_mint` and `is_burn` are derived, not reported: `from_address` is the zero
address, and `to_address` is the zero address. ERC-1155 encodes mints and burns
that way, so the zero address is not an account that sent or received anything.

Without those columns, `GROUP BY from_address` reports the zero address as the
busiest sender on the chain, and a holder count counts it as a holder.

They are two columns rather than one enum because they are independent facts
about a row, and a single column would have to discard one of them.

## Filtering

ERC-1155 indexes `operator` first, so `from` and `to` sit one topic slot later
than in ERC-20 and ERC-721:

| topic | ERC-20 / ERC-721 | ERC-1155 |
| :- | :- | :- |
| topic1 | `from` | `operator` |
| topic2 | `to` | `from` |
| topic3 | token id (ERC-721) | `to` |

`--from-address` and `--to-address` reach topic2 and topic3 accordingly, so
they mean what they say. `--topic1` is the operator.

```bash
# every 1155 movement in a range
triodion erc1155_transfers -b 18M:+1000

# one collection
triodion erc1155_transfers -b 18M:+1000 \
    --address 0x76be3b62873462d2142405439777e971754e8e77

# mints only, on any collection
triodion erc1155_transfers -b 18M:+1000 \
    --from-address 0x0000000000000000000000000000000000000000
```

This dataset is not a member of the `log_events` group. Requesting it
alongside `logs` or `erc20_transfers` therefore costs a separate
`eth_getLogs` per block range rather than sharing one with them.
