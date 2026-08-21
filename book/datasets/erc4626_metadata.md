# erc4626_metadata

One row per (vault, block): the three reads that describe an ERC-4626 tokenised
vault at that block.

```
erc4626_metadata
────────────────
- can collect by block and not by transaction
- required parameters: address
- optional parameters: [none]
- dataset aliases: [none]
- parameter aliases: 
    - contracts -> addresses


schema for erc4626_metadata
───────────────────────────
- block_number: uint32
- erc4626: binary
- asset: binary
- total_assets_binary: binary
- total_assets_string: string
- total_assets_f64: float64
- total_supply_binary: binary
- total_supply_string: string
- total_supply_f64: float64
- chain_id: uint64

sorting erc4626_metadata by: erc4626, block_number

other available columns: [none]
```

Run `triodion help erc4626_metadata` to print this from the binary.

`--blocks` defaults to `latest`.

## The two tokens

`erc4626` is the vault, which is itself a token: the share token. `asset` is the
token it holds, read from `asset()`.

They have their own decimals, and the two need not agree. Nothing in this table
scales one into the other.

`total_assets` is in the vault's asset units. `total_supply` is shares
outstanding — **not** the supply of the underlying token.

## Null and zero are different

A null means the vault refused the read: a revert, or no code at that address at
that block. Zero means the vault answered zero.

Zero is a real state. A vault that has just been deployed holds zero assets, and
one that has been fully redeemed has zero shares. Neither is a failed read.

The combination matters: `total_supply` of zero with a non-zero `total_assets`
is the donated-assets state that the ERC-4626 inflation attack exploits. Never
rewrite either as a null.

## There is no share-price column

A price per share is `total_assets / total_supply`. Both operands are already in
the row, so the column would add no information while forcing three choices on
every reader:

- what to do when `total_supply` is 0 — a fresh vault, or one whose shares were all redeemed. The
  ratio is undefined, not `1.0`.
- how many decimals to round to.
- which of the two token decimals to scale by.

Any answer picked here is wrong for someone.

`convertToAssets(1e18)` is not the same number either, for vaults that charge an
exit fee, so a single column cannot even name one convention. Divide the two
columns yourself, with your own.

```bash
# sDAI
triodion erc4626_metadata --address 0x83f20f44975d03b1b09e64809b757c47f942beea
```

For the flows into and out of a vault, see
[erc4626_vault_events](./erc4626_vault_events.md).
