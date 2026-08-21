# erc777_transfers

One row per ERC-777 `Sent`, `Minted` or `Burned` event.

```
erc777_transfers
────────────────
- can collect by block or by transaction
- required parameters: [none]
- optional parameters: address, topic1, topic2, topic3
- dataset aliases: [none]
- parameter aliases: 
    - contracts -> addresses


schema for erc777_transfers
───────────────────────────
- block_number: uint32
- transaction_index: uint32
- log_index: uint32
- transaction_hash: binary
- erc777: binary
- event_name: string
- operator: binary
- from_address: binary
- to_address: binary
- amount_binary: binary
- amount_string: string
- amount_f64: float64
- data: binary
- operator_data: binary
- is_operator_send: bool
- chain_id: uint64

sorting erc777_transfers by: block_number, log_index

other available columns: block_hash
```

Run `triodion help erc777_transfers` to print this from the binary.

## Do not union this with erc20_transfers

ERC-777 is a superset of ERC-20. A compliant ERC-777 token emits an ERC-20
`Transfer` **for the same movement** alongside every `Sent`, `Minted` and
`Burned`.

Both logs are real, both are in the receipt, and both are collected —
`Transfer` by [erc20_transfers](./erc20_transfers.md), the richer one here.
They are two views of one movement, not two movements.

So this:

```sql
SELECT ... FROM erc20_transfers
UNION ALL
SELECT ... FROM erc777_transfers   -- WRONG
```

reports every ERC-777 movement twice, and every volume, count and balance
derived from it is wrong by exactly the ERC-777 share of the chain.

Pick one table per token: this one when the operator or the data payloads
matter, `erc20_transfers` when only the movement does. If you must have one
table, anti-join on `(transaction_hash, block_number, erc777)` — never on
`log_index`, because the mirrored pair are two different logs with two
different indices.

The mirroring is a convention, not a consensus rule. A token can emit one
without the other, so neither table is a strict subset of the other.

## event_name decides which columns are null

`event_name` holds the Solidity identifier as the standard writes it —
`Sent`, `Minted`, `Burned` — so the value joins straight to an ABI without a
translation table. It also decides which address column is null:

| event_name | from_address | to_address | is_operator_send |
| :- | :- | :- | :- |
| `Sent` | the holder | the recipient | `operator != from_address` |
| `Minted` | null | the recipient | null |
| `Burned` | the holder | null | `operator != from_address` |

A mint has no payer and a burn names no recipient, and ERC-777 does not use the
ERC-20 zero-address convention for either. Writing `0x0` there would invent a
party the event never named.

## operator, and the column that is not it

`operator` is the party that executed the move. This is what ERC-777 adds over
ERC-20: an authorised operator moves a holder's tokens with no allowance and no
call from the holder.

It is always present. On an ordinary self-initiated send the holder is its own
operator, so `COUNT(DISTINCT operator)` counts every ordinary sender as an
operator. `is_operator_send` is the column that separates the two — derived,
not reported, as `operator != from_address`.

It is null on `Minted`, where there is no `from` to compare against. `false`
there would claim a self-send the event never described.

## data and operator_data

Both are non-indexed dynamic `bytes` from the log body, ABI-decoded. `data` is
the holder's payload and `operator_data` is the operator's own, which is why
they are separate columns.

Empty is a real and common value here — most sends carry no payload — so it is
stored as zero-length bytes, never as a null. A null would mean the field does
not exist, and it exists on all three events.

## Filtering by topic, not by name

The three events do not agree on what each topic holds:

| topic | Sent | Minted | Burned |
| :- | :- | :- | :- |
| topic1 | operator | operator | operator |
| topic2 | `from` | `to` | `from` |
| topic3 | `to` | — | — |

So topic1 is offered, and topics 2 and 3 are offered **by position** rather
than under address names. A `--from-address` dim would have to mean topic2, and
would silently match `Minted` recipients too.

Filtering on topic3 also drops every `Minted` and `Burned` row, because those
logs have no topic3 for the node to match against.

This dataset is not a member of the `log_events` group. Requesting it
alongside `logs` or `erc20_transfers` therefore costs a separate
`eth_getLogs` per block range rather than sharing one with them.
