# erc4626_vault_events

One row per ERC-4626 `Deposit` or `Withdraw` event on a tokenised vault.

```
erc4626_vault_events
────────────────────
- can collect by block or by transaction
- required parameters: [none]
- optional parameters: address, topic1
- dataset aliases: erc4626_events
- parameter aliases: 
    - contracts -> addresses


schema for erc4626_vault_events
───────────────────────────────
- block_number: uint32
- transaction_index: uint32
- log_index: uint32
- transaction_hash: binary
- vault: binary
- event_name: string
- sender: binary
- owner: binary
- receiver: binary
- assets_binary: binary
- assets_string: string
- assets_f64: float64
- shares_binary: binary
- shares_string: string
- shares_f64: float64
- chain_id: uint64

sorting erc4626_vault_events by: block_number, log_index

other available columns: block_hash
```

Run `triodion help erc4626_vault_events` to print this from the binary.

Alias: `erc4626_events`.

## Two halves of one movement

```solidity
Deposit (address indexed sender, address indexed owner,    uint256 assets, uint256 shares)
Withdraw(address indexed sender, address indexed receiver, address indexed owner,
         uint256 assets, uint256 shares)
```

Both events are kept in one table because they are the two halves of the same
movement: assets in against shares out, and shares in against assets out.
`event_name` — `"Deposit"` or `"Withdraw"`, spelled as the ABI spells them —
says which half a row is.

`"Withdraw"` is a different event from the `"withdrawal"` of
[erc20_wrapper_events](./erc20_wrapper_events.md). The word is close; the event
is not.

`receiver` is null on every `Deposit` row. `Deposit` has no receiver argument at
all — the shares go to `owner` — so there is nothing to record.

## This is not a WETH deposit

ERC-4626's `Deposit(address,address,uint256,uint256)` is **not** the WETH-style
`Deposit(address,uint256)` that
[erc20_wrapper_events](./erc20_wrapper_events.md) collects.

The two signatures hash to different topic0 values, so the filters here and
there select disjoint sets of logs. A vault deposit can never turn up as a
wrapper deposit. Do not dedupe across the two datasets on that assumption.

## There is no share-price column

The price is `assets / shares`. That division's scaling and rounding belong to
whoever asks the question, and `shares == 0` makes it undefined — a value this
table would then have to invent. Both operands are on the row; divide them with
your own convention.

## Filtering

topic1 is `sender` in both events, so filtering on it means one thing.

topic2 and topic3 are deliberately not offered: topic2 is `owner` on a
`Deposit` but `receiver` on a `Withdraw`, so one filter value would silently
select two different roles.

```bash
triodion erc4626_vault_events -b 18M:+1000 \
    --address 0x83f20f44975d03b1b09e64809b757c47f942beea
```

## What counts as a vault

Matching happens at the event-signature level with no per-contract
configuration. Any contract emitting these exact signatures is indexed, whether
or not it is a conforming vault. Use
[contract_interfaces](./contract_interfaces.md) or
[erc4626_metadata](./erc4626_metadata.md) to check what a given address
actually is.

This dataset is not a member of the `log_events` group. Requesting it
alongside `logs` or `erc20_transfers` therefore costs a separate
`eth_getLogs` per block range rather than sharing one with them.
