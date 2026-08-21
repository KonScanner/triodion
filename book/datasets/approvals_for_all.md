# approvals_for_all

One row per `ApprovalForAll(address,address,bool)` log.

```
approvals_for_all
─────────────────
- can collect by block or by transaction
- required parameters: [none]
- optional parameters: address, topic1, topic2, from_address, to_address
- dataset aliases: erc721_approvals_for_all, erc1155_approvals
- parameter aliases: 
    - contracts -> addresses


schema for approvals_for_all
────────────────────────────
- block_number: uint32
- transaction_index: uint32
- log_index: uint32
- transaction_hash: binary
- contract_address: binary
- owner: binary
- operator: binary
- approved: bool
- chain_id: uint64

sorting approvals_for_all by: block_number, log_index

other available columns: block_hash
```

Run `triodion help approvals_for_all` to print this from the binary.

Aliases: `erc721_approvals_for_all`, `erc1155_approvals`.

## Why the name has no standard in it

ERC-721 and ERC-1155 declare this event with the same argument types:

```solidity
// ERC-721
ApprovalForAll(address indexed owner,   address indexed operator, bool approved)
// ERC-1155
ApprovalForAll(address indexed account, address indexed operator, bool approved)
```

The argument names differ; the types do not. Both therefore hash to one topic0,
and a log that carries it does not say which of the two standards the emitting
contract implements. No decoding recovers that — the bytes are identical.

Calling this dataset `erc1155_approvals` would put a false claim on roughly half
of its rows. Both names are kept as aliases, because both arrive at the same
rows, but neither names the dataset, so no output file claims a standard the
logs cannot prove.

To classify a contract, join `contract_address` against
[contract_interfaces](./contract_interfaces.md), which asks the contract itself
through the ERC-165 `supportsInterface` call.

## What the grant covers

`owner` is the token holder whose permission changes. `operator` is the address
that gains or loses permission over **every** token the owner holds in that
contract.

Unlike the ERC-721 `Approval` event, this grant names no token id, so it also
covers tokens the owner acquires after this block. A blanket grant made once is
still in force against a token bought a year later.

## These are events, not state

`approved == false` is a revocation. It is a measurement of what happened, not
the absence of one, so it is stored as `false` and never as null.

That makes `WHERE approved` wrong on its own. An operator approved in one block
and revoked in the next leaves both rows behind, and the filter keeps the stale
one. Take the latest row per grant first, then filter:

```sql
SELECT * FROM (
    SELECT *, ROW_NUMBER() OVER (
        PARTITION BY contract_address, owner, operator
        ORDER BY block_number DESC, log_index DESC) AS rn
    FROM approvals_for_all
) WHERE rn = 1 AND approved
```

## Filtering

The event carries exactly one signature, so topic1 is the owner and topic2 is
the operator on every row, and address-shaped filtering is unambiguous here.
Both are reachable two ways: as a raw 32-byte topic, or as an ordinary 20-byte
address.

`--from-address` maps to topic1 and `--to-address` to topic2, matching what
[erc20_approvals](./erc20_approvals.md) calls its owner and spender. The names
are loose — an approval has no direction — but demanding a hand-padded 32-byte
word for a value read off a block explorer is worse.

Give one form per slot, not both. `--topic1` together with `--from-address` is
an error rather than a precedence rule: both are partition dimensions, so
resolving the conflict silently would still multiply the run by the discarded
one and write the same rows to several files.

```bash
# every blanket approval a wallet has granted
triodion approvals_for_all -b 18M:+1000 \
    --from-address 0xd8da6bf26964af9d7eed9e03e53415d37aa96045
```

This dataset is not a member of the `log_events` group. Requesting it
alongside `logs` or `erc20_transfers` therefore costs a separate
`eth_getLogs` per block range rather than sharing one with them.
