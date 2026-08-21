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

$EXECUTABLE erc20_approvals -b $BLOCKS -o $OUTPUT_DIR/erc20_approvals

$EXECUTABLE erc20_balances -b $BLOCKS -o $OUTPUT_DIR/erc20_balances --contract 0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2 --address 0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2 &

$EXECUTABLE erc20_metadata -b $BLOCKS -o $OUTPUT_DIR/erc20_metadata --address 0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2 &

$EXECUTABLE erc20_supplies -b $BLOCKS -o $OUTPUT_DIR/erc20_supplies --contract 0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2 &

$EXECUTABLE erc20_transfers -b $BLOCKS -o $OUTPUT_DIR/erc20_transfers

# Deposit / Withdrawal events of a wrapped native token. Aliases: wrapper_events, weth_events
$EXECUTABLE erc20_wrapper_events -b $BLOCKS -o $OUTPUT_DIR/erc20_wrapper_events --address 0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2 &

$EXECUTABLE erc721_metadata -b $BLOCKS -o $OUTPUT_DIR/erc721_metadata --contract 0xed5af388653567af2f388e6224dc7c4b3241c544 &

$EXECUTABLE erc721_transfers -b $BLOCKS -o $OUTPUT_DIR/erc721_transfers --contract 0xed5af388653567af2f388e6224dc7c4b3241c544 &

$EXECUTABLE eth_calls -b $BLOCKS -o $OUTPUT_DIR/eth_calls --call-data 0x18160ddd --contract 0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2 &

$EXECUTABLE logs -b $BLOCKS -o $OUTPUT_DIR/logs

$EXECUTABLE native_transfers -b $BLOCKS -o $OUTPUT_DIR/native_transfers

$EXECUTABLE nonce_diffs -b $BLOCKS -o $OUTPUT_DIR/nonce_diffs

$EXECUTABLE nonce_reads -b $BLOCKS -o $OUTPUT_DIR/nonce_reads

$EXECUTABLE nonces -b $BLOCKS -o $OUTPUT_DIR/nonces --address 0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2 &

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
