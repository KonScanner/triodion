# proxy_slots

One row per (block, address), holding the three ERC-1967 proxy storage slots.

```
proxy_slots
───────────
- can collect by block and not by transaction
- required parameters: address
- optional parameters: [none]
- dataset aliases: erc1967_slots
- parameter aliases: 
    - contracts -> addresses


schema for proxy_slots
──────────────────────
- block_number: uint32
- address: binary
- implementation: binary
- admin: binary
- beacon: binary
- chain_id: uint64

sorting proxy_slots by: block_number, address

other available columns: [none]
```

Run `triodion help proxy_slots` to print this from the binary.

Alias: `erc1967_slots`.

`--blocks` defaults to `latest`.

## The slots

ERC-1967 fixes each slot at `keccak256(label) - 1`:

| column | label | slot |
| :- | :- | :- |
| `implementation` | `eip1967.proxy.implementation` | `0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc` |
| `admin` | `eip1967.proxy.admin` | `0xb53127684a568b3173ae13b9f8a6016e243e63b6e8ee1178d6a717850b5d6103` |
| `beacon` | `eip1967.proxy.beacon` | `0xa3f0ad74e5423aebfd80d3ef4346578335a9a72aeaee59ff6cb3582b35133d50` |

The `- 1` is the point of the construction. It puts the slot outside the image
of `keccak256`, so no mapping or array the contract declares can ever be laid
out on top of it.

## The fourth slot is not read

ERC-1967 defines a rollback slot at
`0x4910fdfa16fed3260ed0e7147f7cc6da11a60208b5b9406d12a635614ffd9143`. It is
deliberately absent.

It is written and cleared inside a single upgrade transaction, so any read at a
block boundary sees it unset. A column of it would be all nulls, and would
invite the reading "no proxy here ever rolled back", which the data cannot
support.

## Null means not that kind of proxy

A null means the slot is unset at that block. Every ordinary, non-proxy contract
answers null in all three columns, which is the expected result for most
addresses.

## implementation and beacon are separate on purpose

- `implementation` set, `beacon` null — a standard ERC-1967 proxy. The code it delegates to is in
  `implementation`.
- `beacon` set — a beacon proxy. Its real implementation is held by the beacon contract, reachable
  only through `IBeacon.implementation()` on that address. It is **not** in this row and must not be
  inferred from it.

The two columns are separate precisely so no single column has to mean both
things.

## The slot view and the event view

This dataset answers "what is the implementation at block N".
[proxy_upgrades](./proxy_upgrades.md) reads the ERC-1967 events and answers
"when did it change, and in which transaction". Join `proxy_slots.address` to
`proxy_upgrades.proxy_address`.

A slot read is the only authority on the current implementation. The events are
a convention, not an EVM rule, so a proxy can write the slot and emit nothing —
and a beacon that swaps its own implementation moves every proxy behind it with
no event at any of them.

```bash
# what does this proxy point at now
triodion proxy_slots --address 0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48

# and how did that change over a range
triodion proxy_slots -b 18M:19M:100000 \
    --address 0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48
```
