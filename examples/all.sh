#!/usr/bin/env bash

# This is an example of collecting most of the triodion datasets
#
# Four datasets are deliberately absent, because they read the consensus layer
# and would fail for anyone running this script without a beacon node:
# `blobs`, `deposit_requests`, `withdrawal_requests` and
# `consolidation_requests`. See examples/blobs.sh and examples/staking.sh.
#
# `withdrawals` IS here. It looks like a beacon dataset and is not: EIP-4895
# withdrawals sit in the execution block body, so an ordinary RPC url serves
# them.

#
# # parameters
#

# use installed triodion installation
EXECUTABLE=triodion

# use local cargo repo
# EXECUTABLE="cargo run --"

# use some other cargo repo
# EXECUTABLE="cargo run --manifest-path MANIFEST_PATH --"

BLOCKS="18M:+100"       # 100 blocks: 18,000,000 to 18,000,099 (range end is exclusive)
SMALL_BLOCKS="18M:+10"  # opcode-level datasets emit millions of rows per block
OUTPUT_DIR="data"       # output directory


#
# # datasets
#

# EIP-2930 access-list entries, one row per storage key. An entry naming an
# account and no keys still gets a row, with a null storage_key.
$EXECUTABLE access_lists -b $BLOCKS -o $OUTPUT_DIR/access_lists

$EXECUTABLE address_appearances -b $BLOCKS -o $OUTPUT_DIR/address_appearances

# ApprovalForAll logs. ERC-721 and ERC-1155 declare this event identically, so
# one topic0 covers both and a log cannot say which standard the contract
# implements. Join contract_address against contract_interfaces to find out.
$EXECUTABLE approvals_for_all -b $BLOCKS -o $OUTPUT_DIR/approvals_for_all

# EIP-7702 authorization tuples. Only type-0x04 transactions carry them, so a
# pre-Prague range yields no rows.
$EXECUTABLE authorizations -b $BLOCKS -o $OUTPUT_DIR/authorizations

$EXECUTABLE balance_diffs -b $BLOCKS -o $OUTPUT_DIR/balance_diffs

$EXECUTABLE balance_reads -b $BLOCKS -o $OUTPUT_DIR/balance_reads

$EXECUTABLE balances -b $BLOCKS -o $OUTPUT_DIR/balances --address 0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2 &

$EXECUTABLE blocks -b $BLOCKS -o $OUTPUT_DIR/blocks

$EXECUTABLE code_diffs -b $BLOCKS -o $OUTPUT_DIR/code_diffs

$EXECUTABLE code_reads -b $BLOCKS -o $OUTPUT_DIR/code_reads

$EXECUTABLE codes -b $BLOCKS -o $OUTPUT_DIR/codes --address 0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2 &

$EXECUTABLE contracts -b $BLOCKS -o $OUTPUT_DIR/contracts

# What a contract answers to ERC-165 supportsInterface, for a fixed table of
# interface ids. Read answers_true_to_everything first: a contract that says yes
# to the reserved id 0xffffffff says yes to everything, so its row is noise.
# Alias: erc165
$EXECUTABLE contract_interfaces --address 0xed5af388653567af2f388e6224dc7c4b3241c544 &

# ERC-1155 URI logs. The event is optional and most contracts never emit it, so
# zero rows is the normal result. Alias: erc1155_uris
$EXECUTABLE erc1155_metadata -b $BLOCKS -o $OUTPUT_DIR/erc1155_metadata

# ERC-1155 transfers, one row per token id moved. A TransferBatch of five ids
# becomes five rows -- an aggregate cannot be taken apart again.
$EXECUTABLE erc1155_transfers -b $BLOCKS -o $OUTPUT_DIR/erc1155_transfers

$EXECUTABLE erc20_approvals -b $BLOCKS -o $OUTPUT_DIR/erc20_approvals

# ERC-20 allowance in force at a block, per (token, owner, spender). This is the
# state; erc20_approvals is the history. transferFrom decrements an allowance
# without emitting Approval, so replaying events overstates what is spendable.
# Alias: allowances
$EXECUTABLE erc20_allowances --contract 0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48 --from-address 0xd8da6bf26964af9d7eed9e03e53415d37aa96045 --to-address 0x68b3465833fb72a70ecdf485e0e4c7bd8665fc45 &

$EXECUTABLE erc20_balances -b $BLOCKS -o $OUTPUT_DIR/erc20_balances --contract 0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2 --address 0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2 &

$EXECUTABLE erc20_metadata -b $BLOCKS -o $OUTPUT_DIR/erc20_metadata --address 0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2 &

$EXECUTABLE erc20_supplies -b $BLOCKS -o $OUTPUT_DIR/erc20_supplies --contract 0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2 &

$EXECUTABLE erc20_transfers -b $BLOCKS -o $OUTPUT_DIR/erc20_transfers

# Deposit / Withdrawal events of a wrapped native token. Aliases: wrapper_events, weth_events
$EXECUTABLE erc20_wrapper_events -b $BLOCKS -o $OUTPUT_DIR/erc20_wrapper_events --address 0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2 &

# ERC-2612 permit nonce, per token per owner. Needs both dimensions to name a
# row. A null nonce means the token has no permit support; 0 means it has permit
# support and this owner has not used it.
$EXECUTABLE erc2612_nonces --contract 0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48 --address 0xd8da6bf26964af9d7eed9e03e53415d37aa96045 &

# ERC-4626 vault asset, total assets and share supply. There is deliberately no
# share-price column: divide the two yourself, with your own convention.
$EXECUTABLE erc4626_metadata --address 0x83f20f44975d03b1b09e64809b757c47f942beea &

# ERC-4626 vault Deposit and Withdraw events. Not the WETH-shape Deposit --
# different signature, different topic0, disjoint from erc20_wrapper_events.
# Alias: erc4626_events
$EXECUTABLE erc4626_vault_events -b $BLOCKS -o $OUTPUT_DIR/erc4626_vault_events

$EXECUTABLE erc721_metadata -b $BLOCKS -o $OUTPUT_DIR/erc721_metadata --contract 0xed5af388653567af2f388e6224dc7c4b3241c544 &

$EXECUTABLE erc721_transfers -b $BLOCKS -o $OUTPUT_DIR/erc721_transfers --contract 0xed5af388653567af2f388e6224dc7c4b3241c544 &

# ERC-777 Sent / Minted / Burned. DO NOT UNION WITH erc20_transfers: a compliant
# ERC-777 token emits an ERC-20 Transfer for the same movement, so both tables
# describe it and a union double-counts every ERC-777 movement on the chain.
$EXECUTABLE erc777_transfers -b $BLOCKS -o $OUTPUT_DIR/erc777_transfers

$EXECUTABLE eth_calls -b $BLOCKS -o $OUTPUT_DIR/eth_calls --call-data 0x18160ddd --contract 0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2 &

$EXECUTABLE logs -b $BLOCKS -o $OUTPUT_DIR/logs

$EXECUTABLE native_transfers -b $BLOCKS -o $OUTPUT_DIR/native_transfers

$EXECUTABLE nonce_diffs -b $BLOCKS -o $OUTPUT_DIR/nonce_diffs

$EXECUTABLE nonce_reads -b $BLOCKS -o $OUTPUT_DIR/nonce_reads

$EXECUTABLE nonces -b $BLOCKS -o $OUTPUT_DIR/nonces --address 0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2 &

# ERC-1967 proxy slots, read from storage. Null in all three columns is the
# normal answer for an ordinary contract. Alias: erc1967_slots
$EXECUTABLE proxy_slots --address 0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48 &

# ERC-1967 proxy events: Upgraded, BeaconUpgraded, AdminChanged. The event view
# of a proxy; proxy_slots is the state view. Neither replaces the other -- a
# proxy can write the slot and emit nothing. Alias: erc1967_events
$EXECUTABLE proxy_upgrades -b $BLOCKS -o $OUTPUT_DIR/proxy_upgrades

# storage slot values. `slots` is the dataset name, `storages` is an alias for it
$EXECUTABLE slots -b $BLOCKS -o $OUTPUT_DIR/slots --address 0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2 --slot 0x0000000000000000000000000000000000000000000000000000000000000000 &

$EXECUTABLE storage_diffs -b $BLOCKS -o $OUTPUT_DIR/storage_diffs

$EXECUTABLE storage_reads -b $BLOCKS -o $OUTPUT_DIR/storage_reads

$EXECUTABLE trace_calls -b $BLOCKS -o $OUTPUT_DIR/trace_calls --call-data 0x18160ddd --contract 0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2 &

$EXECUTABLE traces -b $BLOCKS -o $OUTPUT_DIR/traces

$EXECUTABLE transactions -b $BLOCKS -o $OUTPUT_DIR/transactions

# one row per executed opcode, so use the smaller range. Alias: opcode_traces
$EXECUTABLE vm_traces -b $SMALL_BLOCKS -o $OUTPUT_DIR/vm_traces

# EIP-4895 validator withdrawals, one row each. `blocks` carries only a count
# and a sum, and an aggregate cannot be taken apart again. amount_gwei is gwei,
# not wei -- the column name says so because the mistake is silent.
$EXECUTABLE withdrawals -b $BLOCKS -o $OUTPUT_DIR/withdrawals


#
# # dataset groups
#
# A group name expands to several datasets in one run.

# blocks_and_transactions
$EXECUTABLE blocks transactions -b $BLOCKS -o $OUTPUT_DIR/blocks_transactions

# state_diffs = balance_diffs, code_diffs, nonce_diffs, storage_diffs
$EXECUTABLE state_diffs -b $BLOCKS -o $OUTPUT_DIR/state_diffs

# state_reads = balance_reads, code_reads, nonce_reads, storage_reads
$EXECUTABLE state_reads -b $BLOCKS -o $OUTPUT_DIR/state_reads


#
# # geth datasets
#
# These call debug_traceBlock*. They need a node with the debug namespace
# enabled, which most hosted endpoints do not offer. Skip this section if
# your RPC returns "method not found".

$EXECUTABLE geth_calls -b $SMALL_BLOCKS -o $OUTPUT_DIR/geth_calls

$EXECUTABLE geth_balance_diffs -b $SMALL_BLOCKS -o $OUTPUT_DIR/geth_balance_diffs

$EXECUTABLE geth_code_diffs -b $SMALL_BLOCKS -o $OUTPUT_DIR/geth_code_diffs

$EXECUTABLE geth_nonce_diffs -b $SMALL_BLOCKS -o $OUTPUT_DIR/geth_nonce_diffs

$EXECUTABLE geth_storage_diffs -b $SMALL_BLOCKS -o $OUTPUT_DIR/geth_storage_diffs

$EXECUTABLE geth_opcodes -b $SMALL_BLOCKS -o $OUTPUT_DIR/geth_opcodes

# counts of each function selector called, from debug_traceBlock's 4byteTracer.
# Alias: 4byte_counts
$EXECUTABLE four_byte_counts -b $SMALL_BLOCKS -o $OUTPUT_DIR/four_byte_counts

# javascript_traces runs a tracer you supply. Alias: js_traces
$EXECUTABLE javascript_traces -b $SMALL_BLOCKS -o $OUTPUT_DIR/javascript_traces \
    --js-tracer '{data: [], fault: function(log) {}, step: function(log) { this.data.push(log.op.toString()) }, result: function() { return this.data }}'
