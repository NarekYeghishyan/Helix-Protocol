//! Seeds and spend-limit bounds.

pub const TREASURY_SEED: &[u8] = b"treasury";
pub const VAULT_SEED: &[u8] = b"vault";
pub const VAULT_AUTHORITY_SEED: &[u8] = b"vault_authority";
pub const STREAM_SEED: &[u8] = b"stream";

/// Bounds on the spend-limit epoch.
///
/// A very short epoch makes the per-epoch cap meaningless, since a caller could
/// simply wait for the next window and repeat.
pub const MIN_EPOCH_DURATION: i64 = 3_600;
pub const MAX_EPOCH_DURATION: i64 = 90 * 86_400;

/// A vesting stream must last at least this long.
///
/// Without a floor, a "stream" of one second is just a transfer wearing a
/// vesting schedule's name, which defeats the point of showing a vesting
/// commitment on chain.
pub const MIN_STREAM_DURATION: i64 = 86_400;
