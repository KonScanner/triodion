# erc2612_nonces

One row per (token, owner) per block: the ERC-2612 permit nonce.

```
erc2612_nonces
──────────────
- can collect by block and not by transaction
- required parameters: contract, address
- optional parameters: [none]
- dataset aliases: [none]
- parameter aliases: [none]


schema for erc2612_nonces
─────────────────────────
- block_number: uint32
- erc20: binary
- address: binary
- nonce_binary: binary
- nonce_string: string
- nonce_f64: float64
- chain_id: uint64

sorting erc2612_nonces by: block_number, erc20, address

other available columns: domain_separator
```

Run `triodion help erc2612_nonces` to print this from the binary.

`--blocks` defaults to `latest`. Both `--contract` and `--address` are
required: the nonce is per owner per token, so both dimensions are needed to
name a row.

## Why this dataset exists

ERC-2612 lets a token owner approve a spender with an off-chain signature. The
`permit` call that redeems that signature emits the ordinary ERC-20 `Approval`
event and nothing else — the standard defines no event of its own.

In [erc20_approvals](./erc20_approvals.md) a permit-granted approval is
therefore indistinguishable from one granted by an on-chain `approve()`: same
topic0, same owner, same spender, same value.

The nonce is the only on-chain counter of how many permits an owner has signed
for a token. Read it at two blocks and the difference is the number of permits
redeemed in between.

## What it does not give you

It does not give the linkage. It cannot attribute a specific `Approval` row to a
permit.

Answering that needs the transaction's calldata — the `permit` selector,
possibly nested inside a router's own multicall — which is a different question
from the one asked here.

## Null and zero are different

A successful read of `0` means the owner has signed no permit for this token
yet. That is a real measurement.

A null means the call did not return a nonce at all, so the token has no
ERC-2612 support and the concept does not exist for it.

Filling the null with `0` would report "never permitted" for tokens that can
never permit.

## domain_separator

Off by default. It is the EIP-712 domain separator, and it is constant per token
per chain, so it repeats identically on every row of a run.

Its null is the cheapest signal that a token is not a permit token:
`DOMAIN_SEPARATOR()` reverts on anything that does not implement ERC-2612.

Requesting it costs one extra `eth_call` per row, so it stays opt-in:

```bash
triodion erc2612_nonces \
    --contract 0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48 \
    --address 0xd8da6bf26964af9d7eed9e03e53415d37aa96045 \
    -i domain_separator
```
