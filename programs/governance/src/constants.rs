//! Seeds and governance parameter bounds.

pub const REALM_SEED: &[u8] = b"realm";
pub const PROPOSAL_SEED: &[u8] = b"proposal";
pub const VOTE_SEED: &[u8] = b"vote";

/// Seed for the PDA that signs executed proposals. This is the only signer the
/// treasury accepts, so possession of this PDA *is* the right to spend — and
/// nothing but a passed, timelocked proposal can produce it.
pub const EXECUTOR_SEED: &[u8] = b"executor";

pub const BPS_DENOMINATOR: u128 = 10_000;

/// Bounds on `voting_period`. Long enough that a vote cannot be sprung on
/// holders, short enough to stay governable.
pub const MIN_VOTING_PERIOD: i64 = 3_600; // 1 hour
pub const MAX_VOTING_PERIOD: i64 = 30 * 86_400; // 30 days

/// Bounds on `timelock_delay`.
///
/// The lower bound is not zero. A timelock is the window in which holders who
/// dislike a passed proposal can exit before it takes effect; a zero delay makes
/// the mechanism decorative.
pub const MIN_TIMELOCK_DELAY: i64 = 3_600; // 1 hour
pub const MAX_TIMELOCK_DELAY: i64 = 30 * 86_400; // 30 days

/// A queued proposal must be executed within this window of becoming
/// executable, after which it expires.
///
/// Without an expiry, a proposal passed under one set of conditions could sit
/// dormant for a year and then be executed into a completely different world.
pub const EXECUTION_GRACE_PERIOD: i64 = 14 * 86_400;

/// Bounds on `quorum_bps` and `approval_bps`.
pub const MAX_BPS: u16 = 10_000;
/// A simple majority is the floor for approval — anything less would let a
/// minority carry a proposal against more opposition than support.
pub const MIN_APPROVAL_BPS: u16 = 5_001;

pub const MAX_TITLE_LEN: usize = 64;
pub const MAX_URI_LEN: usize = 200;
