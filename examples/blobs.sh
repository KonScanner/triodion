#!/usr/bin/env bash

# Collect EIP-4844 blobs, and the L1 transactions that paid for them.
#
# The execution layer never carries a blob. An EIP-4844 transaction holds
# only blob_versioned_hashes; the 128 KiB payload lives on the consensus
# layer, in the beacon block's sidecars. So the blobs dataset needs a beacon
# node as well as an execution node, and it emits one row per blob.
#
# The two are joined by hash: each sidecar's KZG commitment hashes to a
# versioned hash, and the transaction whose blob_versioned_hashes contains
# that value is the transaction that paid for the blob. Nothing else links
# them, and the join happens inside triodion — the transaction_hash and
# from_address / to_address columns on a blob row are its result.
#
# Nothing about the clock is compiled in. Genesis time, seconds per slot,
# slots per epoch and the blob schedule are read from /eth/v1/beacon/genesis
# and /eth/v1/config/spec when the beacon connection opens, because mainnet's
# blob maximum has already moved three times since Cancun (6 -> 9 -> 15 ->
# 21) and SECONDS_PER_SLOT differs between networks. Without --beacon-rpc the
# beacon datasets error rather than assume mainnet's clock.
#
# Known limitation: the beacon datasets are indexed by execution block, and
# the slot is derived from the block timestamp as
# (timestamp - genesis_time) / seconds_per_slot. That is exact, but a missed
# slot has no execution block, so a --blocks run cannot see one.

#
# # parameters
#

# use installed triodion installation
EXECUTABLE=triodion

# use local cargo repo
# EXECUTABLE="cargo run --"

export OUTPUT_DIR="data"

# Consensus-layer REST API. Lighthouse and Nimbus default to 5052; Prysm's
# REST API defaults to 3500.
# --beacon-rpc and the BEACON_RPC_URL environment variable are equivalent.
export BEACON_RPC="http://localhost:5052"

# Execution-layer RPC, as usual. Falls back to MESC, then ETH_RPC_URL.
# export ETH_RPC_URL="http://localhost:8545"


#
# # 1. a recent block, from the beacon node alone
#
# A beacon node serves blob sidecars for
# MIN_EPOCHS_FOR_BLOB_SIDECARS_REQUESTS = 4096 epochs, roughly 18 days, and
# then drops them. Inside that window no archive is needed.
#
# Pick a block from the last 18 days. The block below returned 20 blob rows
# when it was recent; ask for it today and the node has already pruned it.

export RECENT_BLOCK="25803013"

$EXECUTABLE blobs \
    --blocks $RECENT_BLOCK \
    --beacon-rpc $BEACON_RPC \
    --output-dir $OUTPUT_DIR/blobs_recent


#
# # 2. a historical block, from a blob archive
#
# Past the retention window the node does not answer with an empty list. A
# public Lighthouse node asked for slot 9,204,782 — execution block
# 20,000,000, June 2024 — answers 403 Forbidden. That is a refusal, not an
# absence, and the two must not be confused, so a beacon failure is only
# swallowed when an archive can answer instead. With no archive configured
# it propagates and the run fails, because "the node refused" must never be
# recorded as "this block posted no blobs".
#
# --blob-archive takes a url, or the word `default`, which is the public
# Blobscan API at https://api.blobscan.com. BLOB_ARCHIVE_URL is equivalent.
# Every row records who answered in blob_source: "beacon_node" or "archive".
#
# This run returns 1 row: slot 9204782, rollup "taiko", blob_source
# "archive", blob_size 131072, blob_used_size 1671, versioned_hash
# 0x017ba4bd9c166498865a3d08618e333ee84812941b5c3a356971b4a6ffffa574,
# transaction_hash
# 0x0ff07f37baa7fa26bb7de3d3fc63002bf0acf3295bdab7f67c108c0d1a3bff15.

export HISTORICAL_BLOCK="20M"

$EXECUTABLE blobs \
    --blocks $HISTORICAL_BLOCK \
    --beacon-rpc $BEACON_RPC \
    --blob-archive default \
    --output-dir $OUTPUT_DIR/blobs_historical \
    --include-columns epoch proposer_index blob_used_size


#
# # 3. the same blobs, joined to the transactions that paid for them
#
# blobs already carries transaction_hash, so collect transactions over the
# same block and join on it. The blob-side columns of an EIP-4844
# transaction are opt-in: blob_versioned_hashes is the list the blobs rows
# match against, and blob_gas_used / blob_gas_price / max_fee_per_blob_gas
# are what the blob space cost.

$EXECUTABLE blobs \
    --blocks $HISTORICAL_BLOCK \
    --beacon-rpc $BEACON_RPC \
    --blob-archive default \
    --output-dir $OUTPUT_DIR/blob_join \
    --subdirs datatype

$EXECUTABLE transactions \
    --blocks $HISTORICAL_BLOCK \
    --output-dir $OUTPUT_DIR/blob_join \
    --subdirs datatype \
    --include-columns \
        blob_versioned_hashes n_blob_versioned_hashes \
        blob_gas_used blob_gas_price max_fee_per_blob_gas


#
# # note on the blob payload
#
# `blob` is an available column and holds the whole 128 KiB payload. A block
# carries at most 21 blobs today, so a full block is about 2.6 MiB and a
# default 1000-block chunk file runs to gigabytes. Add it only when you
# actually need the bytes:
#
#   $EXECUTABLE blobs -b $RECENT_BLOCK --beacon-rpc $BEACON_RPC -i blob
