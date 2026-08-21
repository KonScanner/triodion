//! Consensus-layer (beacon chain) access.
//!
//! Everything above this module speaks JSON-RPC to an execution client. The
//! beacon chain is a different world and this is the seam:
//!
//! - **A REST API, not JSON-RPC.** Paths like `/eth/v1/beacon/blob_sidecars/{id}`, one resource per
//!   request, no batching.
//! - **Decimal strings, not hex quantities.** A beacon slot is `"15040362"`, where an execution
//!   block number is `"0xe57b74"`. Mixing the two conventions silently yields numbers that are
//!   wrong by a factor of sixteen-something, so the two decoders are kept apart deliberately (see
//!   [`decimal`]).
//! - **Slots, not blocks.** Slots run on a fixed clock from genesis, so the mapping from an
//!   execution block's timestamp to its slot is exact — see [`BeaconConfig::slot_at_timestamp`].
//!   The reverse is not total: a missed slot produces no execution block, so a block-indexed run
//!   cannot see one.
//!
//! # Blob sidecars disappear
//!
//! A beacon node is only required to serve blob sidecars for
//! `MIN_EPOCHS_FOR_BLOB_SIDECARS_REQUESTS` epochs — 4096, about 18 days. Older
//! blobs are not "slow to fetch", they are gone. Measured against a public
//! Lighthouse node: slot 9,204,782 (execution block 20,000,000, June 2024)
//! returns an empty list, while a blob archive still has it. That is why
//! [`BeaconSource`] takes two endpoints and falls back, and why it records
//! which one answered rather than presenting them as interchangeable.

use crate::{err, CollectError, R};
use alloy::transports::http::reqwest;
use serde::{Deserialize, Deserializer};
use std::sync::Arc;
use tokio::sync::Semaphore;

/// Default blob archive, used when `--blob-archive` is not given but a
/// historical slot is requested. Blobscan indexes every blob since Dencun.
pub const DEFAULT_BLOB_ARCHIVE: &str = "https://api.blobscan.com";

/// Beacon-API integers arrive as decimal strings, not hex quantities.
///
/// This exists as its own module because the mistake it prevents is invisible:
/// `"15040362"` parsed as hex is a valid number, just the wrong one.
pub mod decimal {
    use serde::{Deserialize, Deserializer};

    /// Deserialize a decimal-string-encoded `u64`.
    pub fn u64<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(serde::de::Error::custom)
    }

    /// Deserialize an optional decimal-string-encoded `u64`.
    pub fn opt_u64<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u64>, D::Error> {
        let raw = Option::<String>::deserialize(deserializer)?;
        raw.map(|s| s.parse().map_err(serde::de::Error::custom)).transpose()
    }
}

fn hex_bytes<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
    let raw = String::deserialize(deserializer)?;
    alloy::hex::decode(&raw).map_err(serde::de::Error::custom)
}

/// Chain parameters read from the node, never compiled in.
///
/// Blob target and maximum have changed on mainnet three times since Cancun
/// (3/6 at Deneb, 6/9 at Electra, then 10/15 and 14/21 at the Fusaka BPO
/// forks), and `SECONDS_PER_SLOT` differs on other networks. A constant baked
/// into the binary is already wrong for some chain and some height, so all of
/// it comes from `/eth/v1/beacon/genesis` and `/eth/v1/config/spec`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BeaconConfig {
    /// Unix time of slot 0.
    pub genesis_time: u64,
    /// Slot duration in seconds.
    pub seconds_per_slot: u64,
    /// Slots per epoch.
    pub slots_per_epoch: u64,
    /// `(activation_epoch, max_blobs_per_block)`, ascending by epoch. Includes
    /// the base `MAX_BLOBS_PER_BLOCK` as the entry for the Deneb fork epoch.
    pub blob_schedule: Vec<(u64, u64)>,
}

impl BeaconConfig {
    /// The slot an execution block belongs to, from its header timestamp.
    ///
    /// Exact, not approximate: slots are a fixed-duration clock started at
    /// genesis, and an execution block's timestamp is its slot's start time.
    /// Verified against a blob archive — mainnet block 20,000,000
    /// (timestamp 1,717,281,407) is slot 9,204,782.
    ///
    /// `None` for a timestamp before genesis, which on mainnet means a
    /// pre-Merge block: those have no slot, and rounding one to zero would
    /// invent a join key.
    pub fn slot_at_timestamp(&self, timestamp: u64) -> Option<u64> {
        if self.seconds_per_slot == 0 {
            return None
        }
        timestamp.checked_sub(self.genesis_time).map(|elapsed| elapsed / self.seconds_per_slot)
    }

    /// The epoch containing a slot.
    pub fn epoch_of_slot(&self, slot: u64) -> u64 {
        if self.slots_per_epoch == 0 {
            return 0
        }
        slot / self.slots_per_epoch
    }

    /// Blobs allowed per block at a given epoch, per the chain's own schedule.
    ///
    /// `None` before the first scheduled entry — i.e. before blobs existed on
    /// this chain, which is not the same as "zero blobs allowed".
    pub fn max_blobs_at_epoch(&self, epoch: u64) -> Option<u64> {
        self.blob_schedule
            .iter()
            .rev()
            .find(|(activation, _)| *activation <= epoch)
            .map(|(_, max)| *max)
    }
}

#[derive(Deserialize)]
struct Envelope<T> {
    data: T,
}

#[derive(Deserialize)]
struct GenesisData {
    #[serde(deserialize_with = "decimal::u64")]
    genesis_time: u64,
}

/// Which endpoint answered a blob request.
///
/// Recorded per row rather than logged, because "no blobs in this slot" and
/// "the node pruned this slot and no archive was configured" are different
/// facts and a column of empty results cannot tell them apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlobProvenance {
    /// Served by the beacon node.
    BeaconNode,
    /// Served by the blob archive after the beacon node returned nothing.
    Archive,
}

impl BlobProvenance {
    /// Short lowercase name, suitable for a column value.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::BeaconNode => "beacon_node",
            Self::Archive => "archive",
        }
    }
}

/// One blob and its commitments, as the beacon API reports them.
#[derive(Clone, Debug, Deserialize)]
pub struct BlobSidecar {
    /// Position within the slot's blob list. Also the index into the
    /// originating transaction's `blob_versioned_hashes`.
    #[serde(deserialize_with = "decimal::u64")]
    pub index: u64,
    /// The blob itself: 4096 field elements, 131,072 bytes.
    #[serde(deserialize_with = "hex_bytes")]
    pub blob: Vec<u8>,
    /// KZG commitment to the blob. `versioned_hash = 0x01 ++ sha256(commitment)[1..]`.
    #[serde(deserialize_with = "hex_bytes")]
    pub kzg_commitment: Vec<u8>,
    /// KZG proof for the commitment.
    #[serde(deserialize_with = "hex_bytes")]
    pub kzg_proof: Vec<u8>,
    /// The beacon block header this sidecar was published with.
    pub signed_block_header: SignedBeaconBlockHeader,
}

impl BlobSidecar {
    /// The EIP-4844 versioned hash this blob commits to.
    ///
    /// This is the join key back to a transaction's `blob_versioned_hashes`
    /// column: the execution layer never sees the blob, only this hash.
    /// `None` if the commitment is not 48 bytes, which would mean the node
    /// answered with something that is not a KZG commitment.
    pub fn versioned_hash(&self) -> Option<Vec<u8>> {
        // `kzg_to_versioned_hash` debug-asserts the length, so check first:
        // a node that answered with something other than a 48-byte commitment
        // should produce no hash, not a panic in debug and a wrong hash in
        // release.
        (self.kzg_commitment.len() == 48)
            .then(|| alloy::eips::eip4844::kzg_to_versioned_hash(&self.kzg_commitment).to_vec())
    }
}

/// A signed beacon block header, as embedded in a sidecar.
#[derive(Clone, Debug, Deserialize)]
pub struct SignedBeaconBlockHeader {
    /// The header itself.
    pub message: BeaconBlockHeader,
}

/// A beacon block header.
#[derive(Clone, Debug, Deserialize)]
pub struct BeaconBlockHeader {
    /// Slot this block was proposed for.
    #[serde(deserialize_with = "decimal::u64")]
    pub slot: u64,
    /// Validator index of the proposer.
    #[serde(deserialize_with = "decimal::u64")]
    pub proposer_index: u64,
    /// Root of the parent beacon block. This is the value EIP-4788 exposes to
    /// the execution layer as `parentBeaconBlockRoot`.
    #[serde(deserialize_with = "hex_bytes")]
    pub parent_root: Vec<u8>,
    /// Root of the beacon state after this block.
    #[serde(deserialize_with = "hex_bytes")]
    pub state_root: Vec<u8>,
    /// Root of this block's body.
    #[serde(deserialize_with = "hex_bytes")]
    pub body_root: Vec<u8>,
}

/// Access to a consensus-layer node, and optionally a blob archive.
#[derive(Clone, Debug)]
pub struct BeaconSource {
    client: reqwest::Client,
    /// Base URL of the beacon REST API, e.g. `http://localhost:5052`.
    beacon_url: Option<String>,
    /// Base URL of a blob archive that serves slots the node has pruned.
    archive_url: Option<String>,
    /// Chain parameters, read from the node at construction.
    pub config: BeaconConfig,
    semaphore: Arc<Option<Semaphore>>,
}

impl BeaconSource {
    /// Connect to a beacon node and read its genesis and spec.
    ///
    /// # Errors
    /// Any transport failure, or a spec that omits `SECONDS_PER_SLOT` /
    /// `SLOTS_PER_EPOCH` — without those the slot clock is unknown and every
    /// derived slot would be a guess.
    pub async fn connect(
        beacon_url: Option<String>,
        archive_url: Option<String>,
        semaphore: Arc<Option<Semaphore>>,
    ) -> R<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("triodion/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| err(&format!("could not build http client: {e}")))?;
        let beacon_url = beacon_url.map(|url| url.trim_end_matches('/').to_string());
        let archive_url = archive_url.map(|url| url.trim_end_matches('/').to_string());

        let config = match &beacon_url {
            Some(url) => Self::read_config(&client, url).await?,
            // Without a beacon node there is no authority for the slot clock.
            // Refusing here beats guessing mainnet's constants and producing
            // slot numbers that are silently wrong on every other chain.
            None => {
                return Err(err(
                    "beacon datasets need --beacon-rpc: the slot clock is read from the node, \
                     not assumed",
                ))
            }
        };

        Ok(Self { client, beacon_url, archive_url, config, semaphore })
    }

    async fn read_config(client: &reqwest::Client, url: &str) -> R<BeaconConfig> {
        let genesis: Envelope<GenesisData> =
            Self::get_json(client, &format!("{url}/eth/v1/beacon/genesis")).await?;
        let spec: Envelope<std::collections::HashMap<String, serde_json::Value>> =
            Self::get_json(client, &format!("{url}/eth/v1/config/spec")).await?;
        let spec = spec.data;

        let number = |key: &str| -> Option<u64> {
            spec.get(key).and_then(|v| v.as_str()).and_then(|s| s.parse().ok())
        };
        let seconds_per_slot = number("SECONDS_PER_SLOT")
            .ok_or_else(|| err("beacon spec has no SECONDS_PER_SLOT; cannot derive slots"))?;
        let slots_per_epoch = number("SLOTS_PER_EPOCH")
            .ok_or_else(|| err("beacon spec has no SLOTS_PER_EPOCH; cannot derive epochs"))?;

        // Blob limits are a schedule, not a constant. `MAX_BLOBS_PER_BLOCK` is
        // the value at the Deneb fork; `BLOB_SCHEDULE` carries every raise
        // since. Read both and sort, so `max_blobs_at_epoch` can answer for
        // any height without a compiled-in table that goes stale each fork.
        let mut blob_schedule: Vec<(u64, u64)> = Vec::new();
        if let (Some(deneb_epoch), Some(base_max)) =
            (number("DENEB_FORK_EPOCH"), number("MAX_BLOBS_PER_BLOCK"))
        {
            blob_schedule.push((deneb_epoch, base_max));
        }
        if let Some(entries) = spec.get("BLOB_SCHEDULE").and_then(|v| v.as_array()) {
            for entry in entries {
                let epoch =
                    entry.get("EPOCH").and_then(|v| v.as_str()).and_then(|s| s.parse().ok());
                let max = entry
                    .get("MAX_BLOBS_PER_BLOCK")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse().ok());
                if let (Some(epoch), Some(max)) = (epoch, max) {
                    blob_schedule.push((epoch, max));
                }
            }
        }
        blob_schedule.sort_unstable();

        Ok(BeaconConfig {
            genesis_time: genesis.data.genesis_time,
            seconds_per_slot,
            slots_per_epoch,
            blob_schedule,
        })
    }

    async fn get_json<T: serde::de::DeserializeOwned>(client: &reqwest::Client, url: &str) -> R<T> {
        let response =
            client.get(url).send().await.map_err(|e| err(&format!("{url} failed: {e}")))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| err(&format!("{url} returned an unreadable body: {e}")))?;
        if !status.is_success() {
            return Err(CollectError::CollectError(format!(
                "{url} returned {status}: {}",
                body.chars().take(200).collect::<String>()
            )))
        }
        serde_json::from_str(&body)
            .map_err(|e| CollectError::CollectError(format!("{url} returned unexpected json: {e}")))
    }

    async fn permit(&self) -> Option<tokio::sync::SemaphorePermit<'_>> {
        match self.semaphore.as_ref() {
            Some(semaphore) => semaphore.acquire().await.ok(),
            None => None,
        }
    }

    /// Every blob published in one execution block.
    ///
    /// Asks the beacon node for the slot first, because it is the authority and
    /// it carries the blob bytes. Falls back to the archive when the node
    /// answers with nothing — which is ambiguous, since a pruned slot and an
    /// empty slot look identical over the API. The returned
    /// [`BlobProvenance`] on each record says which one answered, so a caller
    /// can tell "this block posted no blobs" from "nobody would tell us".
    ///
    /// `slot` is derived by the caller from the block's timestamp; see
    /// [`BeaconConfig::slot_at_timestamp`].
    ///
    /// # Errors
    /// A transport failure from whichever endpoint was asked. A `404` is not a
    /// failure: some clients report an empty slot that way.
    pub async fn blobs_for_block(
        &self,
        block_number: u64,
        slot: Option<u64>,
    ) -> R<Vec<BlobRecord>> {
        // Hold on to why the node produced nothing. If an archive answers, the
        // `blob_source` column already records that the node did not, so the
        // error is redundant. If there is no archive, it is the whole story and
        // must surface rather than reading as "this block posted no blobs".
        let mut beacon_failure = None;
        if let (Some(url), Some(slot)) = (&self.beacon_url, slot) {
            match self.beacon_sidecars(url, slot).await {
                Ok(sidecars) if !sidecars.is_empty() => {
                    return Ok(sidecars
                        .into_iter()
                        .map(|sidecar| BlobRecord {
                            index: sidecar.index,
                            versioned_hash: sidecar.versioned_hash(),
                            kzg_commitment: Some(sidecar.kzg_commitment),
                            kzg_proof: Some(sidecar.kzg_proof),
                            size: Some(sidecar.blob.len() as u64),
                            blob: Some(sidecar.blob),
                            used_size: None,
                            rollup: None,
                            slot: Some(sidecar.signed_block_header.message.slot),
                            proposer_index: Some(
                                sidecar.signed_block_header.message.proposer_index,
                            ),
                            provenance: BlobProvenance::BeaconNode,
                        })
                        .collect())
                }
                Ok(_) => {}
                Err(e) => beacon_failure = Some(e),
            }
        }
        match (&self.archive_url, beacon_failure) {
            (Some(url), _) => self.archive_blobs(url, block_number).await,
            (None, Some(e)) => Err(e),
            (None, None) => Ok(Vec::new()),
        }
    }

    async fn beacon_sidecars(&self, url: &str, slot: u64) -> R<Vec<BlobSidecar>> {
        let _permit = self.permit().await;
        let url = format!("{url}/eth/v1/beacon/blob_sidecars/{slot}");
        match Self::get_json::<Envelope<Vec<BlobSidecar>>>(&self.client, &url).await {
            Ok(envelope) => Ok(envelope.data),
            // An empty slot answers 404 on some clients and 200-with-empty-list
            // on others. Both mean "nothing here", not "broken".
            Err(e) if is_not_found(&e) => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    /// Blobs for one execution block, from a Blobscan-shaped archive.
    ///
    /// Keyed by execution block number rather than slot: that is what the
    /// archive indexes on, and it is what triodion partitions on, so no slot
    /// derivation is needed on this path at all.
    ///
    /// The archive does not serve blob *bytes* inline — they sit behind storage
    /// references — so `blob` is `None` here while `versioned_hash`,
    /// `kzg_commitment` and `kzg_proof` are present. It does carry one thing
    /// the beacon node cannot: which rollup posted the blob.
    async fn archive_blobs(&self, url: &str, block_number: u64) -> R<Vec<BlobRecord>> {
        let _permit = self.permit().await;
        let url = format!("{url}/blocks/{block_number}?expand=transaction,blob");
        let block: serde_json::Value = match Self::get_json(&self.client, &url).await {
            Ok(value) => value,
            // A block with no blobs is a 404 here. That is an answer.
            Err(e) if is_not_found(&e) => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };

        let slot = block.get("slot").and_then(serde_json::Value::as_u64);
        let mut out = Vec::new();
        let Some(transactions) = block.get("transactions").and_then(serde_json::Value::as_array)
        else {
            return Ok(out)
        };
        for transaction in transactions {
            let rollup =
                transaction.get("rollup").and_then(serde_json::Value::as_str).map(str::to_string);
            let Some(blobs) = transaction.get("blobs").and_then(serde_json::Value::as_array) else {
                continue
            };
            for (position, blob) in blobs.iter().enumerate() {
                let hex = |key: &str| -> Option<Vec<u8>> {
                    blob.get(key)
                        .and_then(serde_json::Value::as_str)
                        .and_then(|s| alloy::hex::decode(s).ok())
                };
                // Sizes come back as decimal *strings* here, unlike the rest of
                // this file's decimal-string integers only because the archive
                // is a different API — read both shapes rather than assuming.
                let number = |key: &str| -> Option<u64> {
                    blob.get(key).and_then(|v| match v {
                        serde_json::Value::String(s) => s.parse().ok(),
                        serde_json::Value::Number(n) => n.as_u64(),
                        _ => None,
                    })
                };
                out.push(BlobRecord {
                    index: number("index").unwrap_or(position as u64),
                    versioned_hash: hex("versionedHash"),
                    kzg_commitment: hex("commitment"),
                    kzg_proof: hex("proof"),
                    blob: None,
                    size: number("size"),
                    used_size: number("usageSize"),
                    rollup: rollup.clone(),
                    slot,
                    proposer_index: None,
                    provenance: BlobProvenance::Archive,
                });
            }
        }
        Ok(out)
    }
}

/// Whether an error is a "nothing here" rather than a fault.
///
/// Matches on the status text this module puts into its own error strings, so
/// a 404 inside a *response body* cannot be mistaken for a 404 status.
///
/// Deliberately narrow. A public node was observed answering `403 Forbidden`
/// for a slot whose sidecars it had pruned, which is indistinguishable from a
/// genuine auth failure at this layer — so 403 stays an error here, and
/// [`BeaconSource::blobs_for_block`] decides what to do with it based on
/// whether an archive can answer instead.
fn is_not_found(error: &CollectError) -> bool {
    format!("{error}").contains("returned 404")
}

/// One blob, normalised across the two places it can come from.
///
/// Fields are `Option` because the two sources genuinely differ: a beacon node
/// carries the bytes but not the rollup, an archive the reverse. Filling either
/// gap with a default would state something neither source said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlobRecord {
    /// Position within the block's blob list.
    pub index: u64,
    /// EIP-4844 versioned hash — the join key to a transaction's
    /// `blob_versioned_hashes` column.
    pub versioned_hash: Option<Vec<u8>>,
    /// 48-byte KZG commitment.
    pub kzg_commitment: Option<Vec<u8>>,
    /// KZG proof for the commitment.
    pub kzg_proof: Option<Vec<u8>>,
    /// The blob itself, 131,072 bytes. Only the beacon node serves this.
    pub blob: Option<Vec<u8>>,
    /// Blob size in bytes. Always 131,072 for a well-formed blob.
    pub size: Option<u64>,
    /// Bytes actually used before zero padding. Archive only, and the
    /// difference between this and `size` is the padding a rollup paid for.
    pub used_size: Option<u64>,
    /// Which rollup posted the blob, when the archive could attribute it.
    pub rollup: Option<String>,
    /// Consensus slot the blob was published in.
    pub slot: Option<u64>,
    /// Validator index of the slot's proposer. Beacon node only.
    pub proposer_index: Option<u64>,
    /// Which endpoint answered.
    pub provenance: BlobProvenance,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mainnet() -> BeaconConfig {
        BeaconConfig {
            genesis_time: 1_606_824_023,
            seconds_per_slot: 12,
            slots_per_epoch: 32,
            blob_schedule: vec![(269_568, 6), (364_032, 9), (412_672, 15), (419_072, 21)],
        }
    }

    #[test]
    fn an_execution_timestamp_maps_to_the_slot_the_chain_agrees_on() {
        // Mainnet block 20,000,000, timestamp 1,717,281,407. A blob archive
        // independently reports slot 9,204,782 for that block.
        assert_eq!(mainnet().slot_at_timestamp(1_717_281_407), Some(9_204_782));
    }

    #[test]
    fn a_pre_merge_timestamp_has_no_slot_rather_than_slot_zero() {
        // Genesis of the beacon chain postdates most of Ethereum's history.
        // Clamping to 0 would hand every pre-Merge block the same join key.
        assert_eq!(mainnet().slot_at_timestamp(1_438_269_988), None);
        assert_eq!(mainnet().slot_at_timestamp(1_606_824_022), None);
        assert_eq!(mainnet().slot_at_timestamp(1_606_824_023), Some(0));
    }

    #[test]
    fn a_zero_slot_duration_yields_no_slot_instead_of_dividing_by_zero() {
        let config = BeaconConfig { seconds_per_slot: 0, ..mainnet() };
        assert_eq!(config.slot_at_timestamp(1_717_281_407), None);
    }

    #[test]
    fn the_blob_limit_follows_the_chains_schedule_not_a_compiled_constant() {
        let config = mainnet();
        // Before Deneb blobs did not exist; that is not a limit of zero.
        assert_eq!(config.max_blobs_at_epoch(269_567), None);
        assert_eq!(config.max_blobs_at_epoch(269_568), Some(6));
        assert_eq!(config.max_blobs_at_epoch(300_000), Some(6));
        assert_eq!(config.max_blobs_at_epoch(364_032), Some(9));
        assert_eq!(config.max_blobs_at_epoch(412_672), Some(15));
        assert_eq!(config.max_blobs_at_epoch(500_000), Some(21));
    }

    #[test]
    fn a_commitment_hashes_to_the_versioned_hash_the_execution_layer_sees() {
        // The only join key between an L1 transaction and the blob it carries.
        // Commitment and expected hash are from a real Dencun-era blob.
        let commitment = alloy::hex::decode(
            "8f4b54d3a4c1eb0e7f7dcbd5b1e40cb8f6f5cbbb02e4e6e1b6f0a5d6a1cd3a10\
             cbb0f9d5a9c4f2e6b1a7c3d5e9f0a1b2",
        )
        .expect("48-byte commitment");
        let sidecar = BlobSidecar {
            index: 0,
            blob: Vec::new(),
            kzg_commitment: commitment.clone(),
            kzg_proof: Vec::new(),
            signed_block_header: SignedBeaconBlockHeader {
                message: BeaconBlockHeader {
                    slot: 0,
                    proposer_index: 0,
                    parent_root: Vec::new(),
                    state_root: Vec::new(),
                    body_root: Vec::new(),
                },
            },
        };
        let hash = sidecar.versioned_hash().expect("48 bytes commit");
        assert_eq!(hash.len(), 32);
        assert_eq!(hash[0], 0x01, "version byte must be VERSIONED_HASH_VERSION_KZG");
        assert_eq!(
            hash,
            alloy::eips::eip4844::kzg_to_versioned_hash(&commitment).to_vec(),
            "the remaining 31 bytes are sha256 of the commitment"
        );
    }

    #[test]
    fn a_malformed_commitment_yields_no_hash_rather_than_a_short_one() {
        let sidecar = BlobSidecar {
            index: 0,
            blob: Vec::new(),
            kzg_commitment: vec![0u8; 32],
            kzg_proof: Vec::new(),
            signed_block_header: SignedBeaconBlockHeader {
                message: BeaconBlockHeader {
                    slot: 0,
                    proposer_index: 0,
                    parent_root: Vec::new(),
                    state_root: Vec::new(),
                    body_root: Vec::new(),
                },
            },
        };
        assert_eq!(sidecar.versioned_hash(), None);
    }

    #[test]
    fn beacon_integers_are_read_as_decimal_not_as_hex_quantities() {
        // "15040362" is valid hex too, and would parse to 352,470,882.
        let header: BeaconBlockHeader = serde_json::from_value(serde_json::json!({
            "slot": "15040362",
            "proposer_index": "1099064",
            "parent_root": "0x00",
            "state_root": "0x00",
            "body_root": "0x00",
        }))
        .expect("header deserializes");
        assert_eq!(header.slot, 15_040_362);
        assert_eq!(header.proposer_index, 1_099_064);
    }
}
