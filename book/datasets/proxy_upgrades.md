# proxy_upgrades

One row per ERC-1967 proxy event: `Upgraded`, `BeaconUpgraded` or
`AdminChanged`.

```
proxy_upgrades
──────────────
- can collect by block or by transaction
- required parameters: [none]
- optional parameters: address
- dataset aliases: erc1967_events
- parameter aliases: 
    - contracts -> addresses


schema for proxy_upgrades
─────────────────────────
- block_number: uint32
- transaction_index: uint32
- log_index: uint32
- transaction_hash: binary
- proxy_address: binary
- event_name: string
- implementation: binary
- beacon: binary
- previous_admin: binary
- new_admin: binary
- chain_id: uint64

sorting proxy_upgrades by: block_number, log_index

other available columns: block_hash
```

Run `triodion help proxy_upgrades` to print this from the binary.

Alias: `erc1967_events`.

## One table, three events

```solidity
Upgraded      (address indexed implementation)
BeaconUpgraded(address indexed beacon)
AdminChanged  (address previousAdmin, address newAdmin)
```

`event_name` holds the identifier as the ABI spells it, so a row joins back to
the event definition without a translation table.

Three separate tables would each be near-empty, and would have to be merged
again before "what changed on this proxy, and in what order" could be asked. So
this is one table, and each event fills only its own columns:

| event_name | implementation | beacon | previous_admin | new_admin |
| :- | :- | :- | :- | :- |
| `Upgraded` | set | null | null | null |
| `BeaconUpgraded` | null | set | null | null |
| `AdminChanged` | null | null | set | set |

`proxy_address` is the contract that emitted the event: the proxy itself, never
the implementation it points at.

## Null, not zero

The columns an event does not carry are null. The zero address is a real value
here and must stay distinguishable from an absence:

- a proxy upgraded to `0x0` is bricked
- an admin changed to `0x0` has been renounced

Both are things that happened. A null means the event never named that field at
all.

## What BeaconUpgraded does not tell you

The implementation behind a beacon is not in the log. `BeaconUpgraded` names
the beacon contract; the code it serves must be read from that contract, via
`IBeacon.implementation()`.

## The event view and the slot view

This dataset answers "when did it change, and in which transaction".
[proxy_slots](./proxy_slots.md) reads the same ERC-1967 storage slots directly
and answers "what is the implementation at block N". Join
`proxy_upgrades.proxy_address` to `proxy_slots.address`.

Neither replaces the other:

- These events are a convention, not an EVM rule. A proxy can write the implementation slot and
  emit nothing, and a beacon that swaps its own implementation moves every proxy behind it with no
  event at any of them. **No rows here is not evidence that nothing changed** — only a slot read
  is.
- A slot read cannot say how many times the slot changed between two blocks, or which transaction
  changed it.

## Filtering

Only `--address` is offered. topic1 is deliberately not exposed: it is the
implementation on `Upgraded`, the beacon on `BeaconUpgraded`, and does not exist
on `AdminChanged`. Any `--topic1` value would silently delete every
`AdminChanged` row and mean two different things across the rows it kept.

```bash
# every upgrade of one proxy
triodion proxy_upgrades -b 1:latest \
    --address 0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48
```

This dataset is not a member of the `log_events` group. Requesting it
alongside `logs` or `erc20_transfers` therefore costs a separate
`eth_getLogs` per block range rather than sharing one with them.
