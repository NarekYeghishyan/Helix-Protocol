//! Seeds, fixed-point scale, and the lock-tier table.

/// PDA seeds. Declared once here so the program, the tests and the TypeScript
/// client cannot drift apart through a mistyped literal.
pub const POOL_SEED: &[u8] = b"pool";
pub const POSITION_SEED: &[u8] = b"position";
pub const STAKE_VAULT_SEED: &[u8] = b"stake_vault";
pub const REWARD_VAULT_SEED: &[u8] = b"reward_vault";
pub const VAULT_AUTHORITY_SEED: &[u8] = b"vault_authority";

/// Fixed-point scale for [`crate::state::Pool::reward_per_token`].
///
/// The accumulator holds `rewards * PRECISION / total_weighted`, so `PRECISION`
/// sets how small a per-token increment survives truncation. 1e12 keeps the
/// product `emitted * PRECISION` far below `u128::MAX` for any realistic
/// emission — a full `u64::MAX` of rewards scaled by 1e12 still occupies only
/// about 2^103 — while making the smallest representable increment
/// insignificant next to one lamport of a 9-decimal token.
pub const PRECISION: u128 = 1_000_000_000_000;

/// Denominator for basis-point weights.
pub const BPS_DENOMINATOR: u64 = 10_000;

/// Bounds on `reward_rate`, in reward-token base units per second. The upper
/// bound exists so a mistyped rate cannot overflow the accumulator; the pool
/// would be insolvent long before reaching it anyway.
pub const MAX_REWARD_RATE: u64 = 1_000_000_000_000;

/// Minimum stake, to stop dust positions from being opened purely to consume
/// rent and clutter the indexer.
pub const MIN_STAKE_AMOUNT: u64 = 1_000;

/// Seconds per day, for tier durations.
pub const SECONDS_PER_DAY: i64 = 86_400;
