//! Seeds and bounds. Seeds are `const` rather than inline literals so that the
//! PDA derivations in tests and in the TypeScript client cannot drift from the
//! program by a typo.

/// Seed for the singleton [`crate::state::TokenConfig`].
pub const CONFIG_SEED: &[u8] = b"config";

/// Seed for the PDA that holds the mint authority. No human key ever holds it.
pub const MINT_AUTHORITY_SEED: &[u8] = b"mint_authority";

/// Seed for a [`crate::state::Minter`] registry entry, combined with the
/// authority's public key.
pub const MINTER_SEED: &[u8] = b"minter";

/// Metadata bounds. These cap the on-chain metadata that `initialize_token`
/// writes; they exist so account sizing is a pure function of known constants
/// rather than of caller-controlled input length.
pub const MAX_NAME_LEN: usize = 32;
pub const MAX_SYMBOL_LEN: usize = 10;
pub const MAX_URI_LEN: usize = 200;

/// Default issuance epoch: one day.
pub const DEFAULT_EPOCH_DURATION: i64 = 86_400;

/// Lower bound on an epoch. A very short epoch makes the per-epoch cap
/// meaningless, since a minter could simply wait for the next one.
pub const MIN_EPOCH_DURATION: i64 = 3_600;

/// Upper bound on the number of registered minters, so the registry cannot be
/// grown without limit by an admin.
pub const MAX_MINTERS: u16 = 16;
