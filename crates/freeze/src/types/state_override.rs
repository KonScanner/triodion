//! Bulk state reads through the `eth_call` state-override set.
//!
//! [Multicall3](crate::types::multicall) batches `eth_call`s, but there is no
//! aggregator for `eth_getStorageAt`, `eth_getBalance` or `eth_getCode`: a
//! contract cannot read another contract's storage, and the account-reading
//! opcodes are not exposed through any deployed helper. So a dataset like
//! `slots` or `balances` issues one HTTP request per output row, and a scan of
//! a few thousand slots becomes a few thousand round trips — enough to be
//! rate-limited off most public endpoints before it finishes.
//!
//! Geth's `eth_call` takes an optional third parameter, the *state-override
//! set*: a per-address `{balance, nonce, code, state, stateDiff}` map applied
//! to a scratch copy of the state before the call executes. Overriding `code`
//! lets us run bytecode of our choosing *in the storage context of a real
//! contract*, at a real historical block, without a fork. This is the trick
//! [Dedaub](https://github.com/Dedaub/storage-extractor) published: replace the
//! target's code with a loop that treats calldata as a bare array of 32-byte
//! slot keys, `SLOAD`s each one, and returns the values contiguously. One
//! request then answers thousands of slots.
//!
//! The same loop shape reads accounts rather than storage if the opcode in the
//! middle changes: `BALANCE`, `EXTCODEHASH` and `EXTCODESIZE` all take an
//! address from the stack and push one word. Those three read *arbitrary*
//! accounts, so their extractor does not need to be injected at any particular
//! address — it runs from a scratch address and the calldata names the accounts.
//! See [`StateReader`].
//!
//! # Why every extractor here avoids `PUSH0`
//!
//! Dedaub's published bytecode opens with `PUSH0` (`0x5f`), which saves two
//! bytes. That is correct for their use case — reading *current* storage — and
//! wrong for ours. `eth_call` at a historical block executes under *that
//! block's* fork rules, and `PUSH0` only became valid at Shanghai (EIP-3855,
//! mainnet block 17_034_870, April 2023). Below that height the call does not
//! return wrong data, it fails outright — measured against two independent
//! archive endpoints at block 15_000_000:
//!
//! ```text
//! geth: "invalid opcode: PUSH0"
//! reth: "EVM error: NotActivated"
//! ```
//!
//! An archive-extraction tool spends most of its life below that height, and
//! many L2s enabled Shanghai far later than mainnet did. The `PUSH1 0x00` form
//! is two bytes longer and costs two more gas per *call* (not per element), and
//! it works at every block on every EVM chain. That is the trade this module
//! takes: the fork-independent form always, so there is no fork detection to
//! get wrong and no chain-specific table to keep current.
//!
//! # Verifying the override actually happened
//!
//! Some endpoints accept the third `eth_call` parameter and ignore it. That is
//! the one failure this technique must never wave through: with the override
//! dropped, the calldata is delivered to the *real* contract, which interprets
//! a list of slot keys as a function call. For most contracts that reverts or
//! falls through to a fallback returning empty — but "most" is not "all", and a
//! contract that happened to return 32 bytes would hand back a plausible,
//! entirely fabricated value.
//!
//! [`StateReader::decode_response`] therefore requires the return to be
//! *exactly* `32 * n` bytes, and — for the storage reader — requires the last
//! word to be [`SENTINEL_VALUE`](crate::types::state_override::SENTINEL_VALUE). The extractor
//! returns one word per calldata word by construction, so the length is a positive proof that our
//! code, not the contract's, produced the answer; the sentinel proves it a second time,
//! in band. Anything else is an error, never a partial result.
//!
//! # The policy: on by default, demoted once, never fatal
//!
//! The batch path is attempted automatically rather than hidden behind a flag,
//! because a flag nobody passes is a speedup nobody gets and the thing a flag
//! would guard against is already guarded: every failure lands on the per-row
//! path, which is the code that ran before this module existed. The only real
//! question is what the automatic attempt *costs* against an endpoint that
//! cannot serve it, and the answer has to be "one request for the whole run".
//!
//! [`OverrideSupport`] is what makes that true. It is a run-level memo, shared
//! by every clone of a [`Source`](crate::Source):
//!
//! * The run's first batch attempt is serialised ([`OverrideSupport::gate_first_attempt`]), so an
//!   endpoint that cannot do overrides is discovered by one request rather than by every chunk that
//!   happened to be in flight at startup.
//! * A failure that says *this endpoint does not apply state overrides* ([`override_unavailable`])
//!   latches [`OverrideSupport::rule_out`], and every later group skips the attempt entirely.
//! * Every other failure demotes only the batch in hand, and leaves the memo alone — a timeout says
//!   nothing about whether the endpoint honours overrides.
//!
//! Without the memo the cost is not one wasted request but thousands: the
//! runner halves a failed batch, so a 1000-row group that fails for a reason
//! halving cannot fix spends 1999 doomed `eth_call`s before reaching the
//! per-row path — and then does it again for the next group, and the next.
//! Against a rate-limited endpoint that is not "slower", it is a run that dies
//! of 429s having produced nothing.
//!
//! The memo can only ever *suppress* batching. No state it can reach causes a
//! row to be written from a response [`StateReader::decode_response`] did not
//! verify.

use crate::{CollectError, Params};
use alloy::{
    primitives::{Address, Bytes, B256, U256},
    rpc::types::state::{AccountOverride, StateOverride},
};
use std::sync::atomic::{AtomicU8, Ordering};
use tokio::sync::{Semaphore, SemaphorePermit};

/// Which piece of state one extractor call reads.
///
/// All four share a single 25-byte loop that differs in exactly one opcode, at
/// offset `0x0b`. The loop walks calldata one 32-byte word at a time, applies
/// that opcode to the word, and writes the result to memory at the same offset
/// it read from — so the return is a parallel array: input word `i` produces
/// output word `i`.
///
/// The distinction that matters operationally is *where the code must live*:
///
/// * [`Storage`](Self::Storage) uses `SLOAD`, which only ever reads the storage of the contract
///   currently executing. Its bytecode must be injected **at the address whose storage is wanted**,
///   and one call therefore covers one address. Several addresses need either several calls or a
///   fan-out through Multicall3 / `eth_simulateV1`.
/// * [`Balance`](Self::Balance), [`CodeHash`](Self::CodeHash) and [`CodeSize`](Self::CodeSize) use
///   `BALANCE` / `EXTCODEHASH` / `EXTCODESIZE`, which take the account from the stack. Their
///   bytecode is injected **at a scratch address** ([`SCRATCH_ADDRESS`]) and the calldata names the
///   accounts, so one call covers arbitrarily many unrelated addresses. No real contract is touched
///   at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StateReader {
    /// `SLOAD` — storage slots of the injected-at address. Calldata words are
    /// slot keys.
    Storage,
    /// `BALANCE` — account balances. Calldata words are right-aligned addresses.
    Balance,
    /// `EXTCODEHASH` — account code hashes. Calldata words are right-aligned
    /// addresses. An account with no code hashes to `keccak256("")`; an account
    /// that does not exist hashes to zero.
    CodeHash,
    /// `EXTCODESIZE` — account code lengths in bytes. Calldata words are
    /// right-aligned addresses.
    CodeSize,
}

/// Address the account-reading extractors are injected at.
///
/// `BALANCE` / `EXTCODEHASH` / `EXTCODESIZE` read whatever account the stack
/// names, so the code can run anywhere; it just must not clobber an account the
/// same call needs to read truthfully. This is a fixed, obviously-synthetic
/// address in the range reserved by no chain and occupied on none: overriding
/// it cannot mask a real contract's code, and it is not a precompile (those
/// live at `0x01..=0x11`), so no client's precompile dispatch is disturbed.
///
/// It is deliberately *not* derived from the query. A constant keeps the
/// override set byte-identical between calls, which keeps request bodies
/// comparable in logs and caches.
pub const SCRATCH_ADDRESS: Address = Address::new([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x03, 0x13, 0x37,
]);

/// Storage slot the storage extractor reads as an in-band proof of the override.
///
/// The exact-length check in [`StateReader::decode_response`] catches an
/// endpoint that ignored the override *whenever the real contract answers with
/// a different number of bytes*, which in practice is almost always — most
/// contracts revert or return empty for calldata that is a bare list of slot
/// keys. "Almost always" is not a guarantee, though, and the failure it misses
/// is the worst one available: plausible values, right shape, entirely
/// fabricated, written to a Parquet file that looks correct forever.
///
/// So the storage path also proves the override positively. Alongside `code`
/// it sets `stateDiff { SENTINEL_SLOT: SENTINEL_VALUE }` — a single-slot
/// override, which leaves every other slot exactly as the chain has it — and
/// appends `SENTINEL_SLOT` as the final calldata word. Getting `SENTINEL_VALUE`
/// back in the final returned word means our bytecode ran, the storage override
/// was applied, and the words came back in the order they were sent. All three
/// are the properties the decoder depends on.
///
/// The slot is a fixed nothing-up-my-sleeve constant rather than a per-process
/// random value. That is enough for the realistic threat — a provider that
/// silently drops the third `eth_call` parameter — which cannot conspire to
/// return this exact word. It is *not* proof against a contract deliberately
/// written to mimic the sentinel while overrides are being ignored; such a
/// contract would have to be one the user chose to query, on such an endpoint.
/// A per-process random sentinel would close even that, at the cost of a `rand`
/// dependency this crate does not otherwise carry.
///
/// Chance of colliding with a slot a caller actually wants: 2^-256. Doing it on
/// purpose costs one copied constant, though, because this is public and
/// documented — so the collision is guarded rather than argued away. A row that
/// asks for this slot takes the per-call path, which reads it with no override
/// in sight. See [`StateReader::shadows_sentinel`].
pub const SENTINEL_SLOT: B256 = B256::new([
    0x7d, 0xed, 0xa1, 0x75, 0x0b, 0x1e, 0xe5, 0x10, 0x7e, 0x5c, 0xa1, 0x1e, 0xd0, 0x0b, 0xa5, 0xe5,
    0x10, 0xad, 0xed, 0xc0, 0xde, 0x5e, 0x77, 0x1e, 0x51, 0x0b, 0x5e, 0x77, 0x1e, 0x1a, 0xbe, 0x15,
]);

/// Value [`SENTINEL_SLOT`] is overridden to, and must read back as.
pub const SENTINEL_VALUE: B256 = B256::new([
    0x0e, 0xda, 0xc0, 0xde, 0x15, 0xfa, 0x11, 0xed, 0xba, 0xd0, 0x5e, 0x77, 0x1e, 0x0b, 0xad, 0xc0,
    0xde, 0xfa, 0xce, 0x0f, 0xf1, 0xce, 0x0d, 0xd5, 0x0d, 0xdb, 0xa1, 0x1a, 0xdd, 0x1e, 0x5a, 0xfe,
]);

/// Addresses below this are treated as reserved and never used as an extractor
/// target.
///
/// Overriding the `code` of a precompile is undefined across clients: some
/// dispatch the precompile before consulting the override and run the builtin
/// anyway, which would return a well-formed answer that is not our extractor's.
/// Ethereum's precompiles occupy `0x01..=0x11` and Arbitrum adds ArbOS builtins
/// around `0x64..=0x6f`, but the set grows with every fork and differs per
/// chain, so this reserves the whole low range instead of tracking the exact
/// membership. No real contract lives there; a row naming one falls back to the
/// per-call path, which reads it correctly.
const RESERVED_ADDRESS_CEILING: u64 = 0x1_0000;

/// Offset of the swappable opcode inside [`TEMPLATE`].
const OPCODE_OFFSET: usize = 0x0b;

/// The extractor loop, with a placeholder at [`OPCODE_OFFSET`].
///
/// ```text
/// [00] PUSH1 0x00     cursor = 0                 stack: [cursor]
/// [02] JUMPDEST   <-- loop
/// [03] DUP1                                      stack: [cursor, cursor]
/// [04] CALLDATASIZE                              stack: [size, cursor, cursor]
/// [05] EQ             cursor == size?            stack: [done?, cursor]
/// [06] PUSH1 0x14     -> done
/// [08] JUMPI                                     stack: [cursor]
/// [09] DUP1                                      stack: [cursor, cursor]
/// [0a] CALLDATALOAD   word at cursor             stack: [word, cursor]
/// [0b] <OPCODE>       SLOAD / BALANCE / ...      stack: [value, cursor]
/// [0c] DUP2                                      stack: [cursor, value, cursor]
/// [0d] MSTORE         mem[cursor] = value        stack: [cursor]
/// [0e] PUSH1 0x20
/// [10] ADD            cursor += 32               stack: [cursor]
/// [11] PUSH1 0x02     -> loop
/// [13] JUMP
/// [14] JUMPDEST   <-- done
/// [15] CALLDATASIZE   return length == input length
/// [16] PUSH1 0x00
/// [18] RETURN
/// ```
///
/// The cursor is reused as the memory write offset, which is what makes the
/// loop this short: the value for input word `i` lands at memory offset `32*i`,
/// so the return is already in input order with no bookkeeping.
///
/// Termination is `EQ`, not `LT`, so calldata whose length is not a multiple of
/// 32 would step straight past the end and loop until it ran out of gas.
/// [`StateReader::calldata`] is the only constructor of this calldata and it
/// only ever emits whole words, so the condition cannot arise; the encoder is
/// the guard, not the bytecode.
const TEMPLATE: [u8; 25] = [
    0x60, 0x00, // [00] PUSH1 0x00
    0x5b, // [02] JUMPDEST  <- loop
    0x80, // [03] DUP1
    0x36, // [04] CALLDATASIZE
    0x14, // [05] EQ
    0x60, 0x14, // [06] PUSH1 0x14  -> done
    0x57, // [08] JUMPI
    0x80, // [09] DUP1
    0x35, // [0a] CALLDATALOAD
    0x54, // [0b] <OPCODE>, patched per StateReader
    0x81, // [0c] DUP2
    0x52, // [0d] MSTORE
    0x60, 0x20, // [0e] PUSH1 0x20
    0x01, // [10] ADD
    0x60, 0x02, // [11] PUSH1 0x02  -> loop
    0x56, // [13] JUMP
    0x5b, // [14] JUMPDEST  <- done
    0x36, // [15] CALLDATASIZE
    0x60, 0x00, // [16] PUSH1 0x00
    0xf3, // [18] RETURN
];

impl StateReader {
    /// The opcode this reader substitutes at [`OPCODE_OFFSET`].
    const fn opcode(self) -> u8 {
        match self {
            Self::Storage => 0x54,  // SLOAD
            Self::Balance => 0x31,  // BALANCE
            Self::CodeHash => 0x3f, // EXTCODEHASH
            Self::CodeSize => 0x3b, // EXTCODESIZE
        }
    }

    /// The extractor bytecode for this reader.
    pub fn bytecode(self) -> Bytes {
        let mut code = TEMPLATE;
        code[OPCODE_OFFSET] = self.opcode();
        Bytes::from(code.to_vec())
    }

    /// Whether the bytecode must be injected at the address being read.
    ///
    /// True only for [`Storage`](Self::Storage): `SLOAD` has no address
    /// operand. The account readers run from [`SCRATCH_ADDRESS`].
    pub const fn injects_at_target(self) -> bool {
        matches!(self, Self::Storage)
    }

    /// Encode a batch of inputs as the extractor's calldata.
    ///
    /// The layout is a bare contiguous array of 32-byte big-endian words: no
    /// selector, no ABI header, no length prefix. The loop derives the element
    /// count from `CALLDATASIZE`, so the encoding carries no redundancy — which
    /// is why a slot costs 32 calldata bytes here and ~68 through any
    /// ABI-encoded aggregator.
    ///
    /// For the account readers the caller passes addresses widened to `U256`,
    /// which right-aligns them in the word. The address-taking opcodes mask
    /// their operand to the low 160 bits, so the upper 12 bytes are ignored;
    /// this encoder zeroes them regardless so request bodies stay comparable.
    pub fn calldata(self, inputs: impl IntoIterator<Item = U256>) -> Bytes {
        let mut out: Vec<u8> = Vec::new();
        for word in inputs {
            out.extend_from_slice(&word.to_be_bytes::<32>());
        }
        Bytes::from(out)
    }

    /// Whether this reader proves its override with an in-band sentinel.
    ///
    /// Only the storage reader can: the sentinel is a storage slot, and only
    /// [`Storage`](Self::Storage) reads storage. The account readers do not
    /// need one — they are sent to [`SCRATCH_ADDRESS`], which holds no code, so
    /// an endpoint that dropped the override would be calling a codeless
    /// account and would return empty data, which the length check rejects.
    pub const fn uses_sentinel(self) -> bool {
        self.injects_at_target()
    }

    /// Build the state-override set that makes an extractor call work.
    ///
    /// `target` is the contract whose storage is wanted for
    /// [`Storage`](Self::Storage), and is ignored by the account readers, which
    /// always override [`SCRATCH_ADDRESS`].
    ///
    /// `code` is always set. `state_diff` is set only for the storage reader,
    /// and only for [`SENTINEL_SLOT`]. It is deliberately never `state`: `state`
    /// replaces the account's *entire* storage, so a call made with it would
    /// return a well-formed file of zeros — the exact silent corruption this
    /// module exists to prevent. `state_diff` overrides the named slot and
    /// leaves every other one as the chain has it.
    pub fn overrides(self, target: Address) -> StateOverride {
        let account = AccountOverride {
            code: Some(self.bytecode()),
            state_diff: self
                .uses_sentinel()
                .then(|| [(SENTINEL_SLOT, SENTINEL_VALUE)].into_iter().collect()),
            ..Default::default()
        };
        let mut over = StateOverride::default();
        over.insert(self.call_target(target), account);
        over
    }

    /// Build the complete request for one batch: calldata, overrides, and the
    /// address to send it to.
    ///
    /// The sentinel word, when this reader uses one, is appended *last* so the
    /// caller's inputs keep their indices and
    /// [`Self::decode_response`] can check the tail without disturbing the
    /// parallel-array property the decoder relies on.
    pub fn request(self, target: Address, inputs: &[U256]) -> (Bytes, StateOverride, Address) {
        let words = inputs
            .iter()
            .copied()
            .chain(self.uses_sentinel().then(|| U256::from_be_bytes(SENTINEL_SLOT.0)));
        (self.calldata(words), self.overrides(target), self.call_target(target))
    }

    /// Why this target cannot be used with this reader, if it cannot.
    ///
    /// Two cases, both of which must fall back to the per-call path rather than
    /// producing a value:
    ///
    /// * A reserved low address (see `RESERVED_ADDRESS_CEILING`) — overriding a precompile's code
    ///   is undefined across clients.
    /// * The scratch address, for the account readers (see [`Self::shadows_scratch`]).
    pub fn refuses_target(self, target: Address) -> Option<&'static str> {
        if self.shadows_scratch(target) {
            return Some("address is where the extractor itself runs");
        }
        if self.injects_at_target() && is_reserved_address(target) {
            return Some("address is in the reserved precompile range");
        }
        None
    }

    /// The address an extractor call should be sent `to`.
    ///
    /// The same address the code is injected at: for
    /// [`Storage`](Self::Storage) that is the contract under inspection, so
    /// `SLOAD` resolves against its storage; for the account readers it is
    /// [`SCRATCH_ADDRESS`].
    pub const fn call_target(self, target: Address) -> Address {
        if self.injects_at_target() {
            target
        } else {
            SCRATCH_ADDRESS
        }
    }

    /// Whether this reader would report on its own injected code rather than
    /// on the chain, for `address`.
    ///
    /// The account readers run *from* [`SCRATCH_ADDRESS`], so that address is
    /// the one account in the world whose code they cannot see truthfully:
    /// `EXTCODESIZE(SCRATCH_ADDRESS)` returns 25 — the extractor's own length —
    /// and `EXTCODEHASH` returns `keccak256(extractor)`. Measured against a live
    /// node, which is how this was found: the batch answered correctly for two
    /// real accounts and reported `0x19` for the scratch address.
    ///
    /// The collision is vanishingly unlikely and completely silent, which is
    /// exactly the combination worth a guard. Callers must route a colliding
    /// row through the per-call path instead of the batch.
    ///
    /// [`Storage`](Self::Storage) is never affected: its code is injected at
    /// the target, so the address it reads and the address it runs at are the
    /// same by construction, and `SLOAD` reads storage — which the `code`
    /// override leaves untouched.
    pub fn shadows_scratch(self, address: Address) -> bool {
        !self.injects_at_target() && address == SCRATCH_ADDRESS
    }

    /// Whether this reader would report its own sentinel rather than the chain,
    /// for the row whose calldata word is `word`.
    ///
    /// The mirror of [`Self::shadows_scratch`] on the storage side, and it
    /// closes the same kind of hole. The storage reader writes
    /// `SENTINEL_VALUE` into [`SENTINEL_SLOT`] through `stateDiff`, so
    /// [`SENTINEL_SLOT`] is the one slot in the world it cannot read
    /// truthfully: a caller who asks for it is handed our own marker back.
    ///
    /// Nothing downstream can catch that. The return is exactly `32 * n` bytes,
    /// the tail word is `SENTINEL_VALUE`, and the length check and the sentinel
    /// check both pass — so the row would be written to Parquet as though it
    /// were measured. This is precisely the silent corruption the sentinel
    /// exists to prevent, arriving through the sentinel itself.
    ///
    /// A collision by chance is a `2^-256` event. A collision on purpose is one
    /// copied constant away, because [`SENTINEL_SLOT`] is public and
    /// documented. Callers must route a colliding row through the per-call
    /// path, where `eth_getStorageAt` reads the slot with no override in sight.
    pub fn shadows_sentinel(self, word: U256) -> bool {
        self.uses_sentinel() && word == U256::from_be_bytes(SENTINEL_SLOT.0)
    }

    /// Split an extractor return into one word per input, verifying the
    /// override actually took effect.
    ///
    /// # Errors
    /// Returns [`CollectError::CollectError`] unless the return is *exactly*
    /// `32 * expected` bytes.
    ///
    /// The strictness is the safety property, not fussiness. A shorter return
    /// is the signature of an endpoint that accepted the third `eth_call`
    /// parameter and discarded it: the calldata reached the real contract,
    /// which answered as best it could. Truncating or zero-filling to the
    /// expected length would turn that into silently fabricated storage values
    /// written to a Parquet file, indistinguishable from real ones forever
    /// after. An exact-length check makes the override's effect observable in
    /// the response itself, so the caller can demote to the per-call path
    /// instead of trusting it.
    #[allow(clippy::doc_markdown)]
    pub fn decode_response(self, data: &[u8], n_inputs: usize) -> Result<Vec<B256>, CollectError> {
        let expected = n_inputs + usize::from(self.uses_sentinel());
        let want = expected.saturating_mul(32);
        if data.len() != want {
            return Err(CollectError::CollectError(format!(
                "state-override extractor returned {} bytes, expected {} ({} words): \
                 {IGNORED_MARKER}",
                data.len(),
                want,
                expected,
            )));
        }
        // The length is now known to be an exact multiple of 32, so the
        // remainder `as_chunks` returns is empty by construction.
        let mut words: Vec<B256> =
            data.as_chunks::<32>().0.iter().copied().map(B256::new).collect();

        if self.uses_sentinel() {
            // Positive proof, not an inference from the length: our bytecode
            // ran, the storage override was applied, and the words came back in
            // the order they were sent. A contract answering by accident cannot
            // produce this word.
            let sentinel = words.pop().expect("length checked above includes the sentinel");
            if sentinel != SENTINEL_VALUE {
                return Err(CollectError::CollectError(format!(
                    "{SENTINEL_MARKER}: read {sentinel} from the sentinel slot, expected \
                     {SENTINEL_VALUE}. The endpoint accepted the eth_call state override and did \
                     not apply it, so these values did not come from the extractor",
                )));
            }
        }

        Ok(words)
    }
}

// ---------------------------------------------------------------------------
// Knowing when to stop asking
// ---------------------------------------------------------------------------

/// The tail of the message [`StateReader::decode_response`] produces when the
/// return length proves the extractor never ran.
///
/// Matched by [`override_unavailable`] rather than restated there. This is the
/// one place in the crate where an error is classified by its text, and having
/// the producer and the classifier share a `const` is what keeps them from
/// drifting apart the next time somebody rewords the message.
const IGNORED_MARKER: &str = "the endpoint ignored the eth_call state override";

/// As [`IGNORED_MARKER`], for the sentinel check.
const SENTINEL_MARKER: &str = "state-override sentinel mismatch";

/// Whether this failure means the endpoint will not apply state overrides *at
/// all*, as opposed to failing this one batch.
///
/// Two shapes count:
///
/// * The endpoint took the third `eth_call` parameter and discarded it. The calldata then reached
///   the real contract, whose answer had the wrong length or the wrong sentinel — the two things
///   [`StateReader::decode_response`] refuses. This is the dangerous case, the one where the values
///   would have looked plausible, and it is why that decoder must never pad or truncate.
/// * The endpoint refused the parameter outright. Clients word that differently, so the list below
///   is a sample of what has been observed, not a specification.
///
/// Misclassifying costs requests, never rows. An unrecognised failure still
/// demotes *this* batch to the per-row path (see the ladder in
/// `extractor_batch_with_fallback`); all it fails to do is stop the next
/// batch from trying. Correctness never rests on this function — only the
/// request count does.
pub fn override_unavailable(error: &CollectError) -> bool {
    let message = match error {
        CollectError::CollectError(message) => message.clone(),
        CollectError::ProviderError(rpc_err) => match rpc_err.as_error_resp() {
            Some(payload) => payload.message.to_string(),
            // A transport failure — a dropped connection, a timeout — says
            // nothing about whether the endpoint supports overrides. Ruling the
            // feature out on a blip would cost the whole run its speedup.
            None => return false,
        },
        _ => return false,
    };
    let message = message.to_ascii_lowercase();

    [
        IGNORED_MARKER,
        SENTINEL_MARKER,
        // geth built without the override argument: "too many arguments, want at most 2".
        "too many arguments",
        // Hosted endpoints that gate the feature by tier, and clients that
        // simply have not implemented it. Our own override set is one account
        // with 25 bytes of code and one slot, so "state override" appearing in
        // a size complaint is not a thing that happens.
        "state override",
        "stateoverride",
        "state_override",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

/// Run-level memory of whether this endpoint honours the `eth_call`
/// state-override set.
///
/// One of these lives on [`Source`](crate::Source) and is shared by every clone
/// of it, so the discovery cost is paid once per run rather than once per
/// chunk. See the module docs for why that distinction is the difference
/// between "slower" and "dies of 429s".
///
/// It is deliberately not a per-`(chain, url)` global: a `Source` is the
/// endpoint, and a process that opens a second one against a different
/// provider gets its own verdict rather than inheriting a stranger's.
#[derive(Debug)]
pub struct OverrideSupport {
    /// `UNKNOWN` / `SUPPORTED` / `RULED_OUT`.
    state: AtomicU8,
    /// A single permit, held across the run's first attempt.
    first_attempt: Semaphore,
    /// Consecutive batches that failed without saying why, reset by a success.
    ///
    /// [`rule_out`](Self::rule_out) fires on a message that names the override
    /// as the problem. An endpoint that refuses the third `eth_call` parameter
    /// *below* the JSON-RPC layer never produces one — a stripping proxy
    /// answering 400, an HTML error page, a closed connection all arrive as
    /// bare transport errors, which `override_unavailable` does not claim and
    /// `batch_may_shrink_to_fit` does claim. Those land on the halving rung, so
    /// a 1000-row chunk spends 1999 doomed calls reaching singletons, nothing
    /// latches, and the next chunk does it again.
    ///
    /// This counter is the backstop for that shape: enough silent failures in
    /// a row is itself evidence, whatever the wire said.
    consecutive_failures: AtomicU8,
}

/// Nothing has been tried yet: attempt, and record what comes back.
const UNKNOWN: u8 = 0;
/// A batch came back verified. Attempt freely, with no gate.
const SUPPORTED: u8 = 1;
/// The endpoint does not apply overrides. Never attempt again this run.
const RULED_OUT: u8 = 2;

impl Default for OverrideSupport {
    fn default() -> Self {
        Self {
            state: AtomicU8::new(UNKNOWN),
            first_attempt: Semaphore::new(1),
            consecutive_failures: AtomicU8::new(0),
        }
    }
}

impl OverrideSupport {
    /// Whether a batch attempt is still worth a request.
    ///
    /// False only once [`Self::rule_out`] has fired. Callers re-check this
    /// after [`Self::gate_first_attempt`] returns, because the answer can
    /// change while they wait at the gate.
    pub fn worth_attempting(&self) -> bool {
        self.state.load(Ordering::Relaxed) != RULED_OUT
    }

    /// Record that a batch came back verified.
    ///
    /// Cannot resurrect a ruled-out endpoint. A batch that was already in
    /// flight when another one ruled the feature out must not undo that
    /// ruling — it would put the run straight back into the halving cascade
    /// the ruling exists to prevent.
    pub fn record_supported(&self) {
        // A verified batch means the last failures were not about the override.
        self.consecutive_failures.store(0, Ordering::Relaxed);
        let _ =
            self.state.compare_exchange(UNKNOWN, SUPPORTED, Ordering::Relaxed, Ordering::Relaxed);
    }

    /// Latch "this endpoint does not apply state overrides" for the rest of the
    /// run. Irreversible by design; see [`Self::record_supported`].
    pub fn rule_out(&self) {
        self.state.store(RULED_OUT, Ordering::Relaxed);
    }

    /// Silent batch failures in a row before the fast path is abandoned.
    ///
    /// Three, matching `Source::STORAGE_VALUES_MISS_LIMIT` and for the same
    /// reason: one failure against a load-balanced pool means "this request
    /// landed badly", not "this endpoint cannot do it". Three in a row against
    /// a pool that can is unlikely; against one that cannot it costs three
    /// halving cascades total instead of one per chunk for the whole run.
    const SILENT_FAILURE_LIMIT: u8 = 3;

    /// Record a batch that failed without naming the override as the cause,
    /// and rule the endpoint out once enough have.
    ///
    /// Ruling out on this evidence can be wrong, and being wrong is cheap: the
    /// rows are read by the per-row path either way, so the only cost is the
    /// run losing a speedup. Not ruling out is the expensive mistake.
    pub fn record_silent_failure(&self) {
        let previous = self.consecutive_failures.fetch_add(1, Ordering::Relaxed);
        if previous + 1 >= Self::SILENT_FAILURE_LIMIT {
            self.rule_out()
        }
    }

    /// Serialise the run's first batch attempt.
    ///
    /// Returns `Some` only while the verdict is still `UNKNOWN` — the caller
    /// holds the guard across its first request and drops it as soon as that
    /// request returns, whatever it returned. Everyone else waits, then finds
    /// the answer already recorded.
    ///
    /// This is worth one round-trip of startup latency: without it, every chunk
    /// that starts before the first answer arrives pays the full doomed
    /// cascade, and at the default concurrency that is a hundred of them.
    pub async fn gate_first_attempt(&self) -> Option<SemaphorePermit<'_>> {
        if self.state.load(Ordering::Relaxed) != UNKNOWN {
            return None
        }
        // `acquire` fails only on a closed semaphore, and this one is never
        // closed. Treating the impossible error as "no gate" keeps the batch
        // correct (just unserialised) rather than dropping the row.
        let permit = self.first_attempt.acquire().await.ok()?;
        // Somebody else may have settled the question while we waited.
        (self.state.load(Ordering::Relaxed) == UNKNOWN).then_some(permit)
    }
}

/// Whether `address` is in the range reserved for precompiles and chain
/// builtins. See `RESERVED_ADDRESS_CEILING`.
fn is_reserved_address(address: Address) -> bool {
    let bytes = address.into_array();
    // Everything above the low 8 bytes must be zero for the address to be small.
    bytes[..12].iter().all(|b| *b == 0) &&
        u64::from_be_bytes(bytes[12..].try_into().expect("8 bytes")) < RESERVED_ADDRESS_CEILING
}

// ---------------------------------------------------------------------------
// Batched collection
// ---------------------------------------------------------------------------

/// Words per extractor call before the runner splits the batch.
///
/// Chosen from measurement against a public mainnet endpoint rather than from
/// the gas ceiling. The gas ceiling is generous. Per slot:
///
/// ```text
/// cold SLOAD                                     2100
/// loop body, JUMPDEST..JUMP, SLOAD excluded        51
/// calldata, 32 bytes, worst case all non-zero     512
/// memory, amortised: MSTORE writes at 32*i, so      3   (+ n^2/512 across the call)
///                                                ----
///                                                2666
/// ```
///
/// which puts a 50M-gas cap at ~18.5k slots — close to the ~20k measured
/// directly against a post-Berlin block, where not every calldata byte is
/// non-zero.
///
/// Note the 51: it is the *whole* body, not the 22 gas that reaches the
/// loop-exit test. `JUMPI` is the halfway point of the iteration, not the end
/// of it, and everything after it — the `SLOAD`'s operand handling, the store,
/// the cursor advance and the jump back — costs another 29.
///
/// The binding limits in practice arrive earlier and are about *payload*, not
/// gas:
///
/// ```text
///    100 slots ->  0.1s
///  1_000 slots ->  0.2s
///  5_000 slots ->  0.8s
/// 10_000 slots ->  1.5s   (320 KB request, 320 KB response)
/// 20_000 slots ->  HTTP 503
/// ```
///
/// So the useful range tops out around 10k on that endpoint and the marginal
/// return past ~1000 is poor: ten parallel 1000-word calls finish sooner than
/// one 10k-word call, and they degrade gracefully when a provider caps request
/// bodies. 1000 words is a 32 KB request, comfortably inside every body limit
/// we know of, and still cuts the request count by three orders of magnitude.
/// Raise it with `--state-override-batch-size` when pointing at your own node.
pub const DEFAULT_STATE_OVERRIDE_BATCH_SIZE: u32 = 1000;

/// Smallest group worth reading through an extractor.
///
/// The whole win is requests-per-row, and at one row a group costs exactly one
/// request either way — so the override buys nothing and still pays for itself:
/// a 25-byte `code` override plus a sentinel slot in the request body, and an
/// EVM execution on the server where a plain state read would do. Two rows is
/// where one request starts replacing two.
///
/// Rows below the threshold are not dropped; they take the per-call path, the
/// same one they would take on an endpoint without override support.
const MIN_BATCH_GROUP: usize = 2;

/// Trait for `CollectByBlock` datasets that can be read through a state-override
/// extractor.
///
/// The shape mirrors [`MulticallBatchable`](crate::types::multicall::MulticallBatchable):
/// a dataset says how one row maps to one calldata word and how one returned
/// word maps back to a row, and [`state_override_collect_by_block`] does the
/// grouping, batching, splitting and fallback.
///
/// The difference from Multicall3 is what a batch may contain. Multicall3 sends
/// heterogeneous calls, so a batch is just "rows at the same block". An
/// extractor call runs one piece of bytecode with one override set, so a batch
/// is "rows at the same block *and* the same [`Self::target`]". For
/// [`StateReader::Storage`] the target is the contract being read, which is
/// what makes a slot scan group per contract; for the account readers every row
/// shares [`Address::ZERO`] as a target, so a whole block's addresses batch
/// together.
pub trait StateOverrideBatchable: crate::CollectByBlock {
    /// Which extractor this dataset reads through.
    fn reader() -> StateReader;

    /// The address whose code is overridden for this row.
    ///
    /// For [`StateReader::Storage`] this is the contract being read — rows only
    /// batch together when it matches. The account readers ignore it and should
    /// return [`Address::ZERO`] so every row in a block lands in one batch.
    ///
    /// # Errors
    /// Returns `Err` when the row is missing the parameter, which routes it to
    /// the per-call path rather than failing the partition.
    fn target(params: &Params) -> crate::R<Address>;

    /// The 32-byte word this row contributes to the extractor's calldata: a
    /// storage slot key, or a right-aligned address.
    ///
    /// # Errors
    /// As [`Self::target`].
    fn input_word(params: &Params) -> crate::R<U256>;

    /// Build this row's response from the word the extractor returned for it.
    ///
    /// # Errors
    /// Returns `Err` only for unrecoverable encoding bugs — the extractor
    /// cannot report a per-element failure, so there is no revert to map here.
    fn decode_row(params: &Params, value: B256) -> crate::R<Self::Response>;

    /// Words per call for this dataset. Override when a row's read is unusually
    /// expensive; `Query::state_override_batch_size` takes precedence when set.
    fn default_state_override_batch_size() -> u32 {
        DEFAULT_STATE_OVERRIDE_BATCH_SIZE
    }
}

/// Whether a row would read back part of our own override instead of the chain.
///
/// Each reader has exactly one such blind spot, and it is the same blind spot
/// in both cases: the one input for which the override we install *is* the
/// answer. Neither is detectable downstream — both produce a full-length
/// response with a correct sentinel — so both must be caught here, before the
/// row is ever put in a batch.
///
/// * For the account readers the calldata word *is* the address, and the address they run at is
///   [`SCRATCH_ADDRESS`], which wears the extractor's own code (see
///   [`StateReader::shadows_scratch`]).
/// * For the storage reader the calldata word is a slot key, and the one slot it writes is
///   [`SENTINEL_SLOT`] (see [`StateReader::shadows_sentinel`]).
///
/// A colliding row is not dropped. It takes the per-call path, which reads it
/// with no override in sight and so answers it correctly.
fn row_shadows_override(reader: StateReader, word: U256) -> bool {
    if reader.injects_at_target() {
        return reader.shadows_sentinel(word);
    }
    let bytes = word.to_be_bytes::<32>();
    reader.shadows_scratch(Address::from_slice(&bytes[12..]))
}

/// Split `rows` into at most `batch_size`-sized chunks that are all within one
/// row of each other.
///
/// `slice::chunks` puts the whole remainder in the final chunk, so a group of
/// `batch_size * k + 1` rows ends in a chunk of exactly one. A chunk of one
/// takes the full override path — a code override, a `stateDiff`, two calldata
/// words and a server-side EVM execution — to answer a single slot that
/// `eth_getStorageAt` answers in the same one request. That is the trade
/// [`MIN_BATCH_GROUP`] refuses at the group level, reappearing at the chunk
/// boundary.
///
/// Spreading the remainder one row per chunk instead makes every chunk either
/// `len / n_chunks` or one more than that, so the smallest chunk is as large as
/// an even split allows. It cannot be avoided entirely: three rows at
/// `batch_size` two must still be split `2 + 1`.
fn even_chunks<T>(rows: &[T], batch_size: usize) -> Vec<&[T]> {
    let batch_size = batch_size.max(1);
    let n_chunks = rows.len().div_ceil(batch_size).max(1);
    let base = rows.len() / n_chunks;
    let remainder = rows.len() % n_chunks;

    let mut out = Vec::with_capacity(n_chunks);
    let mut start = 0;
    for i in 0..n_chunks {
        // The first `remainder` chunks take one extra row each, which is what
        // keeps every chunk within one of every other.
        let len = base + usize::from(i < remainder);
        out.push(&rows[start..start + len]);
        start += len;
    }
    out
}

/// State-override-batched collection for `D: StateOverrideBatchable`.
///
/// Groups the partition's rows by `(block, target)`, sends one `eth_call` per
/// [`DEFAULT_STATE_OVERRIDE_BATCH_SIZE`]-word chunk, and halves a chunk that
/// fails for a size-shaped reason — reusing the same classifier the Multicall3
/// runner uses, for the same reason: a node that cannot serve the block at all
/// fails identically at every batch size, so splitting only multiplies the
/// traffic against a node that already said no.
///
/// A chunk that has shrunk to one row falls through to
/// [`CollectByBlock::extract`](crate::CollectByBlock::extract), so the dataset's
/// own per-call path decides that row's outcome. That is what makes the whole
/// feature safe to leave on by default: an endpoint that rejects state
/// overrides, or silently ignores them (caught by [`StateReader::decode_response`]'s
/// length check), degrades to exactly today's behaviour instead of producing
/// wrong data or an empty file.
///
/// # Errors
/// Returns `Err` only for unrecoverable conditions — mpsc send failure, a
/// dataset `transform` failure, or a per-call fallback that itself failed.
pub async fn state_override_collect_by_block<D>(
    partition: crate::Partition,
    source: std::sync::Arc<crate::Source>,
    query: std::sync::Arc<crate::Query>,
    inner_request_size: Option<u64>,
) -> crate::R<std::collections::HashMap<crate::Datatype, polars::prelude::DataFrame>>
where
    D: StateOverrideBatchable + Send + Sync + 'static,
    D::Response: Send + 'static,
{
    use crate::{CollectByBlock, CollectError};
    use std::collections::HashMap;

    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    let chain_id = source.chain_id;
    let reader = D::reader();

    let batch_size = if query.state_override_batch_size > 0 {
        query.state_override_batch_size
    } else {
        D::default_state_override_batch_size()
    }
    .max(1) as usize;

    // Group by (block, target). A row that cannot supply either — or that names
    // a part of our own override rather than the chain, the scratch address for
    // the account readers and the sentinel slot for storage — is ineligible and
    // goes down the per-call path, so the partition's schema is preserved even
    // when part of it cannot be batched.
    let mut groups: HashMap<(u64, Address), Vec<Params>> = HashMap::new();
    let mut ineligible: Vec<Params> = Vec::new();
    for p in partition.param_sets(inner_request_size)? {
        match (p.block_number, D::target(&p), D::input_word(&p)) {
            (Some(block), Ok(target), Ok(word))
                if !row_shadows_override(reader, word) &&
                    reader.refuses_target(target).is_none() =>
            {
                groups.entry((block, target)).or_default().push(p)
            }
            _ => ineligible.push(p),
        }
    }

    // A group too small to pay for itself joins the per-call rows rather than
    // sending an override that saves no requests.
    groups.retain(|_, rows| {
        if rows.len() < MIN_BATCH_GROUP {
            ineligible.append(rows);
            false
        } else {
            true
        }
    });

    // Flatten to the unit of work — one `(block, target, rows)` chunk — so the
    // native reader can be offered several contracts at once. The extractor
    // cannot be: injected code runs at one address and reads only that
    // address's storage, which is why the grouping above is per contract.
    let mut work: Vec<(u64, Address, Vec<Params>)> = Vec::new();
    for ((block, target), rows) in groups {
        for chunk in even_chunks(&rows, batch_size) {
            work.push((block, target, chunk.to_vec()));
        }
    }
    let merge = reader.injects_at_target() && source.storage_values_worth_trying();

    let mut handles = Vec::new();
    for items in plan_native_chunks(work, merge) {
        let sender = sender.clone();
        let source = source.clone();
        let query = query.clone();
        let handle = tokio::task::spawn(async move {
            let responses = native_chunk_with_fallback::<D>(items, &source, query).await?;
            for resp in responses {
                sender
                    .send(Ok(resp))
                    .await
                    .map_err(|_| CollectError::CollectError("mpsc send failed".to_string()))?;
            }
            Ok::<(), CollectError>(())
        });
        handles.push(handle);
    }

    for p in ineligible {
        let sender = sender.clone();
        let source = source.clone();
        let query = query.clone();
        let handle = tokio::task::spawn(async move {
            let resp = D::extract(p, source, query).await?;
            sender
                .send(Ok(resp))
                .await
                .map_err(|_| CollectError::CollectError("mpsc send failed".to_string()))?;
            Ok::<(), CollectError>(())
        });
        handles.push(handle);
    }

    drop(sender);

    let columns = <D as CollectByBlock>::transform_channel(receiver, &query).await?;
    crate::collect_generic::join_partition_handles(handles).await?;
    columns.create_dfs(&query.schemas, chain_id)
}

/// Get every row of one batch out, batched if the endpoint allows it and
/// row-by-row if it does not.
///
/// The ladder, in order:
///
/// 1. **Ruled out already.** [`OverrideSupport`] says this endpoint does not apply overrides, so
///    spend no request finding out again — straight to the per-row path.
/// 2. **Verified batch.** [`StateReader::decode_response`] accepted the length and the sentinel, so
///    the rows come from it and the endpoint is recorded as supporting overrides.
/// 3. **The endpoint does not do overrides** ([`override_unavailable`]). Latch that for the run and
///    read every outstanding row — this batch *and* whatever is still on the stack — one at a time.
///    No batch size fixes this, so halving would only multiply doomed requests.
/// 4. **Size-shaped failure, more than one row.** Halve it. This is the one failure a smaller
///    request can actually fix.
/// 5. **Anything else.** Read these rows one at a time.
///
/// Rung 5 is why this runner cannot turn a working per-row query into a failed
/// one. It replaces an earlier `return Err(e)`, which meant that an endpoint
/// rejecting the override argument in words this module did not recognise
/// failed the whole partition — the one outcome a pure speed change must never
/// produce. A node that genuinely cannot serve the block still fails, but it
/// fails from the per-row call, with the per-row error, exactly as it did
/// before this module existed.
///
/// # Errors
/// Only what the dataset's own [`extract`](crate::CollectByBlock::extract)
/// returns. A batch failure is a routing decision, never an outcome.
async fn extractor_batch_with_fallback<D>(
    block: u64,
    target: Address,
    batch: Vec<Params>,
    source: &std::sync::Arc<crate::Source>,
    query: std::sync::Arc<crate::Query>,
) -> crate::R<Vec<D::Response>>
where
    D: StateOverrideBatchable,
{
    use crate::types::multicall::batch_may_shrink_to_fit;

    let support = &source.state_override_support;
    if !support.worth_attempting() {
        return per_row::<D>(batch, source, &query).await
    }

    // The run's first attempt is serialised so that discovering an endpoint
    // cannot do this costs one request, not one per chunk in flight.
    let mut gate = support.gate_first_attempt().await;
    if !support.worth_attempting() {
        // Another task ruled the endpoint out while this one waited. Release
        // the permit before the fallback, not after: tasks parked in
        // `first_attempt.acquire()` only wake on release, so holding it across
        // a whole per-row batch would stall every one of them for the length
        // of that batch. The gate exists so nothing waits on it longer than a
        // single request.
        drop(gate.take());
        return per_row::<D>(batch, source, &query).await
    }

    let mut stack: Vec<Vec<Params>> = vec![batch];
    let mut out: Vec<D::Response> = Vec::new();
    while let Some(current) = stack.pop() {
        // The verdict can flip mid-cascade — by this task's own silent-failure
        // count, or by another task's. Draining here is what bounds the cost:
        // without it a 1000-row chunk keeps halving to singletons after the
        // endpoint has already been ruled out, which is the 1999-doomed-call
        // run this backstop exists to prevent.
        if !support.worth_attempting() {
            let mut remaining = current;
            for pending in std::mem::take(&mut stack) {
                remaining.extend(pending);
            }
            out.extend(per_row::<D>(remaining, source, &query).await?);
            return Ok(out)
        }

        let result = extractor_batch::<D>(block, target, &current, source).await;
        // Whatever that said, the run now has evidence. Release the gate before
        // interpreting it so the other chunks are not held behind a halving
        // cascade that only concerns this one.
        drop(gate.take());

        match result {
            Ok(responses) => {
                support.record_supported();
                out.extend(responses)
            }

            Err(ref e) if override_unavailable(e) => {
                support.rule_out();
                let mut remaining = current;
                for pending in std::mem::take(&mut stack) {
                    remaining.extend(pending);
                }
                out.extend(per_row::<D>(remaining, source, &query).await?);
                return Ok(out)
            }

            Err(ref e) if current.len() > 1 && batch_may_shrink_to_fit(e) => {
                support.record_silent_failure();
                let mid = current.len() / 2;
                let mut left = current;
                let right = left.split_off(mid);
                // Push right first so left is popped next — keeps produced rows
                // in roughly input order. The frame is sorted by schema at the
                // end regardless, so this is legibility, not correctness.
                stack.push(right);
                stack.push(left);
            }

            Err(_) => {
                support.record_silent_failure();
                out.extend(per_row::<D>(current, source, &query).await?)
            }
        }
    }
    Ok(out)
}

/// Read rows the way the dataset always has: one RPC per row.
///
/// Every row is put in flight at once rather than awaited in sequence, so
/// `Source`'s semaphore is what caps the concurrency — the same thing that caps
/// it when the per-row collector spawns a task per row. Demoting a batch
/// therefore costs requests, not serialisation: a run against an endpoint
/// without override support takes as long as it took before this module
/// existed, not `n` times longer.
///
/// # Errors
/// The first per-row failure, as the per-row collector would have reported it.
async fn per_row<D>(
    rows: Vec<Params>,
    source: &std::sync::Arc<crate::Source>,
    query: &std::sync::Arc<crate::Query>,
) -> crate::R<Vec<D::Response>>
where
    D: StateOverrideBatchable,
{
    futures::future::try_join_all(
        rows.into_iter().map(|p| D::extract(p, source.clone(), query.clone())),
    )
    .await
}

/// One extractor call for one `(block, target)` batch.
async fn extractor_batch<D>(
    block: u64,
    target: Address,
    batch: &[Params],
    source: &std::sync::Arc<crate::Source>,
) -> crate::R<Vec<D::Response>>
where
    D: StateOverrideBatchable,
{
    let reader = D::reader();
    let mut words: Vec<U256> = Vec::with_capacity(batch.len());
    for p in batch {
        words.push(D::input_word(p)?);
    }

    // The native reader is tried in `native_storage_batch`, before this
    // function is reached, and across every contract at the block rather than
    // one at a time. Retrying it here would spend a second request to be told
    // the same thing.
    let (call_data, overrides, to) = reader.request(target, &words);
    let output = source.call_with_overrides(to, call_data.into(), overrides, block).await?;
    let values = reader.decode_response(&output, batch.len())?;

    batch.iter().zip(values).map(|(p, v)| D::decode_row(p, v)).collect()
}

/// Slots per `eth_getStorageValues` request.
///
/// geth answers `-38026 "too many slots (max 1024)"` above this, and the limit
/// counts every slot in the request — across all the contracts named in it, not
/// per contract. It is one budget for the whole map.
const NATIVE_STORAGE_MAX_SLOTS: usize = 1024;

/// Group the work items into requests the native reader can actually serve.
///
/// `eth_getStorageValues` takes a `{contract: [slots]}` map, so slots for
/// *different* contracts at the same block travel in one request. That is the
/// one thing the extractor cannot do at any batch size: injected code runs at a
/// single address and reads only that address's storage. A partition of 500
/// contracts × 2 slots is 500 extractor calls and one native call.
///
/// When `merge` is false — the account readers, or an endpoint already known
/// not to have the method — every item becomes its own group and the runner
/// behaves exactly as it did before this function existed.
fn plan_native_chunks(
    work: Vec<(u64, Address, Vec<Params>)>,
    merge: bool,
) -> Vec<Vec<(u64, Address, Vec<Params>)>> {
    if !merge {
        return work.into_iter().map(|item| vec![item]).collect()
    }

    // Sorted by block so a partition's requests come out in a stable order;
    // the frame is sorted by schema at the end, so this is legibility only.
    let mut by_block: std::collections::BTreeMap<u64, Vec<(u64, Address, Vec<Params>)>> =
        std::collections::BTreeMap::new();
    for item in work {
        by_block.entry(item.0).or_default().push(item);
    }

    let mut out = Vec::new();
    for (_, items) in by_block {
        let mut current: Vec<(u64, Address, Vec<Params>)> = Vec::new();
        let mut budget = 0usize;
        for item in items {
            let n = item.2.len();
            // An item already over the ceiling on its own is left alone.
            // Merging could only make the request larger, and its refusal would
            // then be counted against the endpoint as a missing method.
            if n > NATIVE_STORAGE_MAX_SLOTS {
                out.push(vec![item]);
                continue
            }
            if !current.is_empty() && budget + n > NATIVE_STORAGE_MAX_SLOTS {
                out.push(std::mem::take(&mut current));
                budget = 0;
            }
            budget += n;
            current.push(item);
        }
        if !current.is_empty() {
            out.push(current);
        }
    }
    out
}

/// Answer a group of same-block work items: natively if the node can, and with
/// the per-contract extractor ladder if it cannot.
///
/// # Errors
/// Only what the extractor ladder returns, which is only what the dataset's own
/// per-row path returns. A native miss is a routing decision, never an outcome.
async fn native_chunk_with_fallback<D>(
    items: Vec<(u64, Address, Vec<Params>)>,
    source: &std::sync::Arc<crate::Source>,
    query: std::sync::Arc<crate::Query>,
) -> crate::R<Vec<D::Response>>
where
    D: StateOverrideBatchable,
{
    if let Some(responses) = native_storage_batch::<D>(&items, source).await {
        return Ok(responses)
    }

    // Each item keeps its own ladder and they run concurrently, so a native
    // miss costs the one request it spent — never the parallelism.
    let per_item = items
        .into_iter()
        .map(|(block, target, rows)| {
            extractor_batch_with_fallback::<D>(block, target, rows, source, query.clone())
        })
        .collect::<Vec<_>>();
    Ok(futures::future::try_join_all(per_item).await?.into_iter().flatten().collect())
}

/// One `eth_getStorageValues` request covering every item in the group.
///
/// This is the rung above the extractor, and it is strictly better where it
/// exists: no `code` override, so none of the override failure modes apply and
/// there is no sentinel to check — the node is answering the question that was
/// asked, not running our bytecode. It landed in geth v1.17.1 and nothing else
/// ships it, so most endpoints fall straight through.
///
/// Returns `None`, never an error, whenever the answer is not complete and
/// in order. "This node does not have the method" is a routing fact rather than
/// a failure, and so is a short or unexpected answer — the caller falls through
/// to the extractor either way, which is what it would have done before this
/// function existed.
async fn native_storage_batch<D>(
    items: &[(u64, Address, Vec<Params>)],
    source: &std::sync::Arc<crate::Source>,
) -> Option<Vec<D::Response>>
where
    D: StateOverrideBatchable,
{
    if !D::reader().injects_at_target() || !source.storage_values_worth_trying() {
        return None
    }
    // `plan_native_chunks` keeps a group under the ceiling, except for a single
    // item that was already over it on its own — which happens when
    // `--state-override-batch-size` is set above 1024. Sending it would buy a
    // guaranteed `-38026` (geth, erigon) or `-32602` (reth). Those do now count
    // as misses, so the run would stop asking after three — but three wasted
    // requests for a limit we already know is a worse answer than not asking.
    if items.iter().map(|(_, _, rows)| rows.len()).sum::<usize>() > NATIVE_STORAGE_MAX_SLOTS {
        return None
    }
    let block = items.first()?.0;

    // One contract can appear in more than one item, when its rows were split
    // by batch size. So the map is *extended* per contract, never inserted
    // into: an insert would drop the earlier item's slots on the floor and the
    // length check below would then reject the whole group.
    let mut request: std::collections::HashMap<Address, Vec<B256>> =
        std::collections::HashMap::new();
    for (_, target, rows) in items {
        let entry = request.entry(*target).or_default();
        for p in rows {
            entry.push(B256::from(D::input_word(p).ok()?.to_be_bytes::<32>()));
        }
    }

    let answer = source.get_storage_values(&request, block).await.ok()?;

    // Take the values back out in the order they went in, walking a per-contract
    // cursor so a contract split across items gets each item's own slice.
    let mut cursors: std::collections::HashMap<Address, usize> = std::collections::HashMap::new();
    let mut out = Vec::with_capacity(items.iter().map(|(_, _, rows)| rows.len()).sum());
    for (_, target, rows) in items {
        let values = answer.get(target)?;
        let start = *cursors.get(target).unwrap_or(&0);
        let end = start.checked_add(rows.len())?;
        if end > values.len() {
            return None
        }
        for (p, v) in rows.iter().zip(&values[start..end]) {
            out.push(D::decode_row(p, *v).ok()?);
        }
        cursors.insert(*target, end);
    }

    // A node that answered with more slots than were asked for, or named a
    // contract that was not in the request, is not answering this question.
    // Refuse the lot rather than pick the plausible parts out of it.
    if answer.len() != request.len() {
        return None
    }
    for (target, values) in &answer {
        if cursors.get(target).copied().unwrap_or(0) != values.len() {
            return None
        }
    }

    Some(out)
}

#[cfg(test)]
mod tests {

    #[test]
    fn chunking_never_leaves_a_lone_row_behind() {
        // `chunks(batch_size)` puts the whole remainder last, so `batch * k + 1`
        // rows ended in a chunk of one — the single-row override call that
        // `MIN_BATCH_GROUP` exists to refuse.
        for batch in [2usize, 3, 10, 50, 100, 250, 1000, 1024] {
            for len in 2..3000usize {
                let rows: Vec<usize> = (0..len).collect();
                let chunks = even_chunks(&rows, batch);

                let sizes: Vec<usize> = chunks.iter().map(|c| c.len()).collect();
                assert_eq!(sizes.iter().sum::<usize>(), len, "len={len} batch={batch}");
                assert!(sizes.iter().all(|n| *n <= batch), "len={len} batch={batch} {sizes:?}");

                // Every chunk within one row of every other is the strongest
                // statement available: three rows at batch two must still be
                // 2 + 1.
                let (min, max) = (
                    *sizes.iter().min().expect("at least one chunk"),
                    *sizes.iter().max().expect("at least one chunk"),
                );
                assert!(max - min <= 1, "len={len} batch={batch} {sizes:?}");

                // The rows come back in order, and all of them come back.
                let flat: Vec<usize> = chunks.concat();
                assert_eq!(flat, rows, "len={len} batch={batch}");
            }
        }
    }

    #[test]
    fn a_group_that_fits_in_one_batch_stays_one_chunk() {
        let rows: Vec<usize> = (0..1000).collect();
        assert_eq!(even_chunks(&rows, 1000).len(), 1);
        assert_eq!(even_chunks(&rows, 1001).len(), 1);
        // One row over is two near-equal chunks, not 1000 + 1.
        let rows: Vec<usize> = (0..1001).collect();
        let sizes: Vec<usize> = even_chunks(&rows, 1000).iter().map(|c| c.len()).collect();
        assert_eq!(sizes, vec![501, 500]);
    }
    use super::*;

    /// Every extractor is the same 25 bytes with one opcode swapped, and the
    /// jump destinations must land on the two `JUMPDEST`s regardless.
    #[test]
    fn bytecode_is_the_template_with_one_opcode_swapped() {
        for (reader, opcode) in [
            (StateReader::Storage, 0x54),
            (StateReader::Balance, 0x31),
            (StateReader::CodeHash, 0x3f),
            (StateReader::CodeSize, 0x3b),
        ] {
            let code = reader.bytecode();
            assert_eq!(code.len(), 25, "{reader:?}");
            assert_eq!(code[OPCODE_OFFSET], opcode, "{reader:?}");
            // Everything except the swapped byte is identical across readers.
            for (i, (got, want)) in code.iter().zip(TEMPLATE.iter()).enumerate() {
                if i != OPCODE_OFFSET {
                    assert_eq!(got, want, "{reader:?} byte {i:#04x}");
                }
            }
            // The two forward jumps must target real JUMPDESTs, or every call
            // reverts with "invalid jump destination".
            assert_eq!(code[0x02], 0x5b, "loop JUMPDEST");
            assert_eq!(code[0x14], 0x5b, "exit JUMPDEST");
            assert_eq!(code[0x07], 0x14, "JUMPI target is the exit JUMPDEST");
            assert_eq!(code[0x12], 0x02, "JUMP target is the loop JUMPDEST");
        }
    }

    /// The published Dedaub bytecode is the `PUSH0` form of the same loop. We
    /// deliberately do not emit it (see the module docs), so this pins the
    /// difference to exactly the two `PUSH0`s and the shifted jump targets —
    /// if someone "optimises" the template back to 23 bytes, this fails.
    #[test]
    fn we_do_not_emit_the_push0_form() {
        let code = StateReader::Storage.bytecode();
        let dedaub = alloy::hex::decode("5f5b80361460135780355481526020016001565b365ff3").unwrap();
        assert_eq!(dedaub.len(), 23);
        assert_ne!(code.as_ref(), dedaub.as_slice());
        assert!(!code.contains(&0x5f), "PUSH0 is invalid below Shanghai");
    }

    #[test]
    fn calldata_is_bare_contiguous_words() {
        let data = StateReader::Storage.calldata([U256::from(0), U256::from(1), U256::from(2)]);
        assert_eq!(data.len(), 96, "no selector, no ABI header, no length prefix");
        assert_eq!(data[31], 0);
        assert_eq!(data[63], 1);
        assert_eq!(data[95], 2);
        // Empty input is empty calldata: the loop exits on the first iteration.
        assert!(StateReader::Storage.calldata([]).is_empty());
    }

    #[test]
    fn addresses_are_right_aligned_in_their_word() {
        let addr = Address::from_slice(&[0xab; 20]);
        let data = StateReader::Balance.calldata([U256::from_be_slice(addr.as_slice())]);
        assert_eq!(data.len(), 32);
        assert_eq!(&data[..12], &[0u8; 12], "upper 12 bytes zeroed");
        assert_eq!(&data[12..], addr.as_slice());
    }

    #[test]
    fn storage_overrides_the_target_and_account_readers_do_not() {
        let target = Address::from_slice(&[0x11; 20]);

        let storage = StateReader::Storage.overrides(target);
        assert!(storage.contains_key(&target));
        assert_eq!(StateReader::Storage.call_target(target), target);

        for reader in [StateReader::Balance, StateReader::CodeHash, StateReader::CodeSize] {
            let over = reader.overrides(target);
            assert!(!over.contains_key(&target), "{reader:?} must not touch a real contract");
            assert!(over.contains_key(&SCRATCH_ADDRESS), "{reader:?}");
            assert_eq!(reader.call_target(target), SCRATCH_ADDRESS);
        }
    }

    /// `state` would replace the account's whole storage — the values this call
    /// exists to read. Only `code` and a single-slot `state_diff` are ever set.
    #[test]
    fn overrides_never_replace_whole_storage() {
        let target = Address::from_slice(&[0x22; 20]);

        let over = StateReader::Storage.overrides(target);
        let account = over.get(&target).expect("target override present");
        assert_eq!(account.code, Some(StateReader::Storage.bytecode()));
        assert!(account.balance.is_none());
        assert!(account.nonce.is_none());
        assert!(account.state.is_none(), "`state` would mask the real storage");
        // The sentinel, and only the sentinel.
        let diff = account.state_diff.as_ref().expect("storage reader sets a sentinel");
        assert_eq!(diff.len(), 1);
        assert_eq!(diff.get(&SENTINEL_SLOT), Some(&SENTINEL_VALUE));

        // The account readers read no storage, so they set no diff at all.
        for reader in [StateReader::Balance, StateReader::CodeHash, StateReader::CodeSize] {
            let over = reader.overrides(target);
            let account = over.get(&SCRATCH_ADDRESS).expect("scratch override present");
            assert!(account.state.is_none(), "{reader:?}");
            assert!(account.state_diff.is_none(), "{reader:?}");
        }
    }

    /// The sentinel rides along as the final calldata word so the caller's
    /// inputs keep their indices.
    #[test]
    fn storage_requests_append_the_sentinel_last() {
        let target = Address::from_slice(&[0x44; 20]);
        let inputs = [U256::from(7), U256::from(8)];

        let (call_data, _over, to) = StateReader::Storage.request(target, &inputs);
        assert_eq!(to, target);
        assert_eq!(call_data.len(), 32 * 3, "two inputs plus the sentinel");
        assert_eq!(&call_data[64..96], SENTINEL_SLOT.as_slice());

        // Account readers carry no sentinel: an ignored override would call a
        // codeless address and return empty, which the length check rejects.
        let (call_data, _over, to) = StateReader::Balance.request(target, &inputs);
        assert_eq!(to, SCRATCH_ADDRESS);
        assert_eq!(call_data.len(), 32 * 2);
    }

    #[test]
    fn decode_splits_the_return_into_words() {
        // Storage: two values then the sentinel.
        let mut data = Vec::new();
        data.extend_from_slice(&[0xaa; 32]);
        data.extend_from_slice(&[0xbb; 32]);
        data.extend_from_slice(SENTINEL_VALUE.as_slice());
        let words = StateReader::Storage.decode_response(&data, 2).unwrap();
        assert_eq!(words, vec![B256::repeat_byte(0xaa), B256::repeat_byte(0xbb)]);

        // Account readers: no sentinel to strip.
        let mut data = Vec::new();
        data.extend_from_slice(&[0xcc; 32]);
        let words = StateReader::Balance.decode_response(&data, 1).unwrap();
        assert_eq!(words, vec![B256::repeat_byte(0xcc)]);
        assert_eq!(StateReader::Balance.decode_response(&[], 0).unwrap(), Vec::<B256>::new());
    }

    /// The load-bearing safety test. An endpoint that accepts the third
    /// `eth_call` parameter and ignores it delivers our calldata to the real
    /// contract, whose answer must never be mistaken for storage.
    #[test]
    fn decode_rejects_a_return_the_override_did_not_produce() {
        // Ignored override, contract fell through to an empty fallback.
        let err = StateReader::Storage.decode_response(&[], 3).unwrap_err();
        assert!(format!("{err:?}").contains("ignored"), "error must name the cause: {err:?}");

        // Wrong length in either direction — never truncate or pad to fit.
        assert!(StateReader::Storage.decode_response(&[0u8; 64], 3).is_err());
        assert!(StateReader::Storage.decode_response(&[0u8; 192], 3).is_err());
        // Not a whole number of words.
        assert!(StateReader::Storage.decode_response(&[0u8; 31], 1).is_err());
    }

    /// The case the length check alone cannot catch: a contract that answers
    /// with exactly the right number of bytes. Only the sentinel separates a
    /// real read from a coincidence.
    #[test]
    fn a_right_sized_return_without_the_sentinel_is_still_rejected() {
        let mut forged = Vec::new();
        forged.extend_from_slice(&[0x11; 32]); // plausible "slot 0"
        forged.extend_from_slice(&[0x22; 32]); // plausible "slot 1"
        forged.extend_from_slice(&[0x00; 32]); // where the sentinel should be
        assert_eq!(forged.len(), 96, "exactly the length the decoder expects");

        let err = StateReader::Storage.decode_response(&forged, 2).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("sentinel"), "must name the sentinel: {msg}");
        assert!(msg.contains("did not apply it"), "must say what went wrong: {msg}");
    }

    /// Overriding a precompile's code is undefined across clients, so those
    /// addresses are never used as extractor targets.
    #[test]
    fn reserved_low_addresses_are_refused_as_storage_targets() {
        for byte in [0x01u8, 0x02, 0x09, 0x11, 0x64, 0x6f, 0xff] {
            let mut raw = [0u8; 20];
            raw[19] = byte;
            let addr = Address::from(raw);
            assert!(is_reserved_address(addr), "{addr} is in the reserved range");
            assert!(StateReader::Storage.refuses_target(addr).is_some(), "{addr}");
        }
        // Just past the ceiling, and an ordinary contract, are both fine.
        let mut raw = [0u8; 20];
        raw[17] = 0x01; // 0x010000
        assert!(!is_reserved_address(Address::from(raw)));
        assert!(StateReader::Storage.refuses_target(Address::from(raw)).is_none());
        assert!(StateReader::Storage.refuses_target(Address::from_slice(&[0xab; 20])).is_none());
        // A high address whose low bytes look small must NOT be mistaken for one.
        let mut high = [0u8; 20];
        high[0] = 0x01;
        high[19] = 0x01;
        assert!(!is_reserved_address(Address::from(high)));
    }

    /// Found by running the emitted bytecode against a live node: the account
    /// readers execute *at* the scratch address, so that one account reports
    /// the extractor's own code (size 25, hash `keccak256(extractor)`) instead
    /// of the chain's. Storage is immune — its code lives at the target and
    /// `SLOAD` reads storage, which the `code` override does not touch.
    #[test]
    fn account_readers_cannot_see_the_scratch_address_truthfully() {
        for reader in [StateReader::Balance, StateReader::CodeHash, StateReader::CodeSize] {
            assert!(reader.shadows_scratch(SCRATCH_ADDRESS), "{reader:?}");
            assert!(!reader.shadows_scratch(Address::from_slice(&[0x33; 20])), "{reader:?}");
        }
        assert!(
            !StateReader::Storage.shadows_scratch(SCRATCH_ADDRESS),
            "storage runs at the target, not the scratch address",
        );
        // The value a shadowed EXTCODESIZE would report, pinned so the guard's
        // reason stays legible: the extractor's own length.
        assert_eq!(StateReader::CodeSize.bytecode().len(), 25);
    }

    /// The storage-side mirror of the test above, and the more dangerous of the
    /// two: `SENTINEL_SLOT` is the one slot the storage reader writes, so a row
    /// asking for it would be answered with `SENTINEL_VALUE` — our own marker,
    /// returned as though it were measured.
    ///
    /// Nothing downstream can catch it. The response is exactly `32 * n` bytes
    /// and its tail word is `SENTINEL_VALUE`, so the length check and the
    /// sentinel check both pass and the row is written out. The guard has to be
    /// at the runner's filter, which is what this pins.
    #[test]
    fn the_storage_reader_cannot_see_the_sentinel_slot_truthfully() {
        let sentinel = U256::from_be_bytes(SENTINEL_SLOT.0);
        assert!(StateReader::Storage.shadows_sentinel(sentinel));
        assert!(row_shadows_override(StateReader::Storage, sentinel));

        // Any other slot, including the neighbouring keys, batches as usual.
        for other in [U256::ZERO, U256::from(1), sentinel - U256::from(1), sentinel + U256::from(1)]
        {
            assert!(!row_shadows_override(StateReader::Storage, other), "{other}");
        }

        // The account readers write no storage, so the sentinel is not their
        // blind spot — the scratch address is, and that guard still stands.
        for reader in [StateReader::Balance, StateReader::CodeHash, StateReader::CodeSize] {
            assert!(!reader.shadows_sentinel(sentinel), "{reader:?}");
            assert!(!row_shadows_override(reader, sentinel), "{reader:?}");
            let scratch = U256::from_be_slice(SCRATCH_ADDRESS.as_slice());
            assert!(row_shadows_override(reader, scratch), "{reader:?}");
        }
    }

    /// Pin the per-slot gas budget that `DEFAULT_STATE_OVERRIDE_BATCH_SIZE`'s
    /// derivation rests on, so an edit to `TEMPLATE` cannot silently invalidate
    /// it.
    ///
    /// The 51 is the whole loop body. An earlier version of that doc said 22,
    /// which is the prefix that reaches `JUMPI` — the loop-exit test is the
    /// middle of the iteration, not the end of it.
    #[test]
    fn the_extractor_loop_costs_fifty_one_gas_per_slot() {
        // (mnemonic, gas) for offsets 0x02..=0x13 of TEMPLATE: one iteration,
        // with the SLOAD at 0x0b excluded because it is the swapped opcode and
        // its cost is fork-dependent.
        let body = [
            ("JUMPDEST", 1),
            ("DUP1", 3),
            ("CALLDATASIZE", 2),
            ("EQ", 3),
            ("PUSH1", 3),
            ("JUMPI", 10),
            ("DUP1", 3),
            ("CALLDATALOAD", 3),
            ("DUP2", 3),
            ("MSTORE", 3),
            ("PUSH1", 3),
            ("ADD", 3),
            ("PUSH1", 3),
            ("JUMP", 8),
        ];
        let per_iteration: u32 = body.iter().map(|(_, g)| g).sum();
        assert_eq!(per_iteration, 51, "loop body cost changed; fix the batch-size derivation");

        // The prefix that stops at the loop-exit test, kept so the off-by-29 is
        // named rather than merely absent.
        let through_jumpi: u32 = body.iter().take(6).map(|(_, g)| g).sum();
        assert_eq!(through_jumpi, 22);

        // 2100 cold SLOAD + 51 loop + 512 worst-case calldata + 3 memory.
        let per_slot = 2100 + per_iteration + 512 + 3;
        assert_eq!(per_slot, 2666);
        // Which leaves the 50M cap comfortably above the 1000-word default.
        assert!(u64::from(DEFAULT_STATE_OVERRIDE_BATCH_SIZE) * u64::from(per_slot) < 50_000_000);
    }

    // -----------------------------------------------------------------------
    // The demotion policy
    // -----------------------------------------------------------------------

    /// The classifier must recognise what the decoder actually produces, not a
    /// paraphrase of it. Both errors are built by calling `decode_response`, so
    /// rewording either message without updating the marker fails here rather
    /// than silently costing every run its speedup.
    #[test]
    fn the_classifier_recognises_what_the_decoder_produces() {
        // An endpoint that dropped the override: the real contract fell through
        // to an empty fallback, so the return is the wrong length.
        let ignored = StateReader::Storage.decode_response(&[], 3).unwrap_err();
        assert!(override_unavailable(&ignored), "{ignored:?}");

        // An endpoint that dropped the override and whose contract happened to
        // answer with the right *number* of bytes. Only the sentinel catches
        // this one, and it must classify the same way.
        let mut plausible = vec![0u8; 32 * 3];
        plausible.extend_from_slice(&[0u8; 32]); // the sentinel slot read back as zero
        let mismatch = StateReader::Storage.decode_response(&plausible, 3).unwrap_err();
        assert!(override_unavailable(&mismatch), "{mismatch:?}");
    }

    /// A failure that is about size, or about the node, must NOT rule the
    /// feature out — halving fixes the first and nothing here fixes the second,
    /// but neither says the endpoint refuses overrides. Ruling out on these
    /// would cost every run its speedup after one unlucky request.
    #[test]
    fn the_classifier_ignores_failures_that_are_not_about_overrides() {
        for message in [
            "request entity too large",
            "execution reverted",
            "missing trie node: state is not available",
            "your app has exceeded its compute units per second capacity",
        ] {
            let err = CollectError::CollectError(message.to_string());
            assert!(!override_unavailable(&err), "{message}");
        }
    }

    /// An opcode the block's fork does not have must land on the per-call path,
    /// and on nothing else.
    ///
    /// `EXTCODEHASH` arrived at Constantinople (block 7280000) and `PUSH0` at
    /// Shanghai, so an old enough block rejects the extractor outright. The
    /// right response is to read those rows one at a time. The two wrong ones
    /// are both reachable by accident:
    ///
    /// * `override_unavailable` would latch `rule_out()` for the whole process, so one historical
    ///   chunk would cost every later dataset its batching.
    /// * `batch_may_shrink_to_fit` would halve the batch down to singletons, each one failing the
    ///   same way — the doomed cascade the demotion policy exists to bound.
    ///
    /// Neither message contains any of the needles today. This pins that, so a
    /// needle added later for some other endpoint cannot quietly poison the
    /// fork path.
    #[test]
    fn an_unsupported_opcode_is_neither_a_missing_feature_nor_a_size_complaint() {
        use crate::types::multicall::batch_may_shrink_to_fit;
        for message in [
            "invalid opcode: opcode 0x3f not defined",
            "invalid opcode: PUSH0",
            "EVM error: NotActivated",
        ] {
            let err = CollectError::CollectError(message.to_string());
            assert!(!override_unavailable(&err), "would latch for the run: {message}");
            // `batch_may_shrink_to_fit` returns true for a non-provider error by
            // design — one smaller attempt is cheap. What matters is that it is
            // not a *provider* error saying "too large", so check the shape the
            // node actually sends.
            let rpc = CollectError::ProviderError(alloy::transports::RpcError::ErrorResp(
                alloy::rpc::json_rpc::ErrorPayload {
                    code: -32000,
                    message: std::borrow::Cow::Owned(message.to_string()),
                    data: None,
                },
            ));
            assert!(!override_unavailable(&rpc), "would latch for the run: {message}");
            assert!(!batch_may_shrink_to_fit(&rpc), "would halve pointlessly: {message}");
        }
    }

    /// The memo starts optimistic, latches once, and cannot be talked back out
    /// of it by a batch that was already in flight when the verdict landed.
    #[test]
    fn the_memo_latches_one_way() {
        let support = OverrideSupport::default();
        assert!(support.worth_attempting(), "an untried endpoint is worth one request");

        support.record_supported();
        assert!(support.worth_attempting());

        support.rule_out();
        assert!(!support.worth_attempting());

        // The in-flight batch that started before the ruling comes back OK.
        // Honouring it would put the run straight back into the cascade the
        // ruling exists to end.
        support.record_supported();
        assert!(!support.worth_attempting(), "a ruled-out endpoint must stay ruled out");
    }

    /// The gate opens for the first caller and stays shut for everyone else
    /// once a verdict exists — that is the whole "one wasted request per run,
    /// not one per chunk" property.
    #[tokio::test]
    async fn the_gate_admits_the_first_attempt_and_then_stands_aside() {
        let support = OverrideSupport::default();

        let first = support.gate_first_attempt().await;
        assert!(first.is_some(), "the first attempt of a run is gated");
        // The first attempt reports back and releases the gate.
        support.record_supported();
        drop(first);

        assert!(
            support.gate_first_attempt().await.is_none(),
            "with a verdict recorded there is nothing left to serialise",
        );

        let ruled_out = OverrideSupport::default();
        ruled_out.rule_out();
        assert!(ruled_out.gate_first_attempt().await.is_none());
    }

    // ---------------------------------------------------------------
    // Cross-contract planning for the native reader
    // ---------------------------------------------------------------

    /// `n` rows at one block against one contract. The rows' contents do not
    /// matter to the planner — only how many there are.
    fn item(block: u64, target: u8, n: usize) -> (u64, Address, Vec<Params>) {
        let mut address = [0u8; 20];
        address[19] = target;
        let rows = (0..n)
            .map(|i| Params {
                block_number: Some(block),
                address: Some(address.to_vec()),
                slot: Some(U256::from(i).to_be_bytes::<32>().to_vec()),
                ..Default::default()
            })
            .collect();
        (block, Address::new(address), rows)
    }

    fn shape(plan: &[Vec<(u64, Address, Vec<Params>)>]) -> Vec<Vec<usize>> {
        plan.iter().map(|g| g.iter().map(|(_, _, rows)| rows.len()).collect()).collect()
    }

    /// With merging off — an account reader, or an endpoint already known not
    /// to have the method — the plan must be exactly what the runner did before
    /// any of this existed: one request per contract per block.
    #[test]
    fn the_planner_is_a_no_op_when_merging_is_off() {
        let work = vec![item(10, 1, 3), item(10, 2, 4), item(11, 1, 5)];
        assert_eq!(shape(&plan_native_chunks(work, false)), vec![vec![3], vec![4], vec![5]]);
    }

    /// The whole point: different contracts at the same block share a request.
    #[test]
    fn the_planner_merges_contracts_that_share_a_block() {
        let work = vec![item(10, 1, 3), item(10, 2, 4), item(10, 3, 5)];
        let plan = plan_native_chunks(work, true);
        assert_eq!(plan.len(), 1, "one block, one request");
        assert_eq!(shape(&plan), vec![vec![3, 4, 5]]);
    }

    /// A request names one block, so two blocks can never share one however
    /// small they are. Merging across them would read the wrong state.
    #[test]
    fn the_planner_never_merges_across_blocks() {
        let work = vec![item(10, 1, 1), item(11, 1, 1), item(12, 1, 1)];
        let plan = plan_native_chunks(work, true);
        assert_eq!(plan.len(), 3);
        for group in &plan {
            let blocks: std::collections::BTreeSet<u64> =
                group.iter().map(|(b, _, _)| *b).collect();
            assert_eq!(blocks.len(), 1);
        }
    }

    /// The ceiling counts every slot in the request, across contracts. Three
    /// 400-slot contracts are 1200 slots, which geth refuses, so the group must
    /// break before it gets there.
    #[test]
    fn the_planner_respects_the_shared_slot_ceiling() {
        let work = vec![item(10, 1, 400), item(10, 2, 400), item(10, 3, 400)];
        let plan = plan_native_chunks(work, true);
        assert_eq!(shape(&plan), vec![vec![400, 400], vec![400]]);
        for group in &plan {
            let slots: usize = group.iter().map(|(_, _, rows)| rows.len()).sum();
            assert!(slots <= NATIVE_STORAGE_MAX_SLOTS, "{slots} slots in one request");
        }
    }

    /// An item already over the ceiling on its own — the user raised
    /// `--state-override-batch-size` past 1024 — must not drag neighbours into
    /// a request that was going to be refused anyway. Its refusal would be
    /// counted against the endpoint as a missing method.
    #[test]
    fn the_planner_isolates_an_item_that_is_already_too_large() {
        let work = vec![item(10, 1, NATIVE_STORAGE_MAX_SLOTS + 1), item(10, 2, 2)];
        assert_eq!(shape(&plan_native_chunks(work, true)), vec![vec![1025], vec![2]]);
    }

    /// Every row must survive planning exactly once. A planner that dropped or
    /// duplicated an item would produce a short or double-counted frame, which
    /// no downstream length check would catch.
    #[test]
    fn the_planner_conserves_every_row() {
        let work = vec![
            item(10, 1, 700),
            item(10, 2, 700),
            item(11, 1, 2),
            item(10, 3, 1),
            item(11, 2, NATIVE_STORAGE_MAX_SLOTS + 5),
        ];
        let before: usize = work.iter().map(|(_, _, rows)| rows.len()).sum();
        let plan = plan_native_chunks(work, true);
        let after: usize = plan.iter().flatten().map(|(_, _, rows)| rows.len()).sum();
        assert_eq!(before, after);
    }
}
