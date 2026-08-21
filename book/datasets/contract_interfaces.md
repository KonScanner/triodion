# contract_interfaces

One row per (block, address): what a contract answers to ERC-165
`supportsInterface(bytes4)` for a fixed table of interface ids.

```
contract_interfaces
───────────────────
- can collect by block and not by transaction
- required parameters: address
- optional parameters: [none]
- dataset aliases: erc165
- parameter aliases: 
    - contracts -> addresses


schema for contract_interfaces
──────────────────────────────
- block_number: uint32
- address: binary
- answers_true_to_everything: bool
- supports_erc165: bool
- supports_erc721: bool
- supports_erc721_metadata: bool
- supports_erc721_enumerable: bool
- supports_erc1155: bool
- supports_erc1155_metadata_uri: bool
- supports_erc2981: bool
- supports_erc1271: bool
- chain_id: uint64

sorting contract_interfaces by: block_number, address

other available columns: [none]
```

Run `triodion help contract_interfaces` to print this from the binary.

Alias: `erc165`.

## What it is for

This is the classification layer the token datasets sit on. An address that
answers `true` for ERC-721 can be collected as an ERC-721, instead of being
guessed at from the shape of its logs.

[approvals_for_all](./approvals_for_all.md) is the clearest case: ERC-721 and
ERC-1155 emit an identical `ApprovalForAll` topic0, so the log cannot say which
standard a contract implements. This dataset asks the contract.

`--blocks` defaults to `latest`, so a run with no block range reads the current
state.

## The interface ids

An interface id is the XOR of the four-byte selectors of every function in the
interface. Each is fixed by its standard:

| column | interface | id |
| :- | :- | :- |
| `supports_erc165` | ERC-165 | `0x01ffc9a7` |
| `supports_erc721` | ERC-721 | `0x80ac58cd` |
| `supports_erc721_metadata` | ERC-721 Metadata | `0x5b5e139f` |
| `supports_erc721_enumerable` | ERC-721 Enumerable | `0x780e9d63` |
| `supports_erc1155` | ERC-1155 | `0xd9b67a26` |
| `supports_erc1155_metadata_uri` | ERC-1155 MetadataURI | `0x0e89341c` |
| `supports_erc2981` | ERC-2981 royalties | `0x2a55205a` |
| `supports_erc1271` | ERC-1271 signatures | `0x1626ba7e` |

## Three states, not two

Every answer is nullable, and the three states are three different facts:

- **null** — the call reverted, the address has no code, or the return was not an ABI `bool`. The
  contract said nothing, which is what a non-ERC-165 address looks like.
- **false** — the contract answered no.
- **true** — the contract answered yes.

Filling null with `false` would turn "never answered" into "answered no", and a
count of `false` would then include every EOA in the input.

## The two columns that judge the rest of the row

`supports_erc165` is required by the standard to be `true`. A contract that
implements `supportsInterface` but answers `false` here is not ERC-165
compliant, and its other answers are its own convention rather than the
standard's.

`answers_true_to_everything` is the probe for the reserved id `0xffffffff`.
ERC-165 **requires** a compliant contract to answer `false` to it. So `true`
here means the contract answers yes to every id ever asked: discard every other
column in the row, none of them carry information.

`null` there is not an all-clear either. It means no decodable answer, so the
contract made no ERC-165 promise about the rest of the row.

Read both before trusting any `supports_*` column:

```sql
SELECT * FROM contract_interfaces
WHERE supports_erc721
  AND supports_erc165
  AND NOT COALESCE(answers_true_to_everything, TRUE)
```

```bash
triodion contract_interfaces \
    --address 0xed5af388653567af2f388e6224dc7c4b3241c544
```
