//! One variant per failure mode.
//!
//! Anchor 1.x permits exactly one `#[error_code]` block per program, so every
//! failure in this crate is named here. Specific errors are not a style
//! preference: when a transaction fails on mainnet, this enum is the only thing
//! the operator has to go on.

use anchor_lang::prelude::*;

#[error_code]
pub enum TokenManagerError {
    #[msg("Caller is not the configured admin")]
    NotAdmin,

    #[msg("No admin transfer is pending")]
    NoPendingAdmin,

    #[msg("Caller is not the pending admin")]
    NotPendingAdmin,

    #[msg("Admin transfer to the current admin is a no-op")]
    AdminUnchanged,

    #[msg("Token operations are paused")]
    Paused,

    #[msg("Minter is not enabled")]
    MinterDisabled,

    #[msg("Mint would exceed this minter's cap for the current epoch")]
    EpochCapExceeded,

    #[msg("Epoch duration is outside the permitted range")]
    InvalidEpochDuration,

    #[msg("The minter registry is full")]
    MinterRegistryFull,

    #[msg("Amount must be greater than zero")]
    ZeroAmount,

    #[msg("Metadata field exceeds its maximum length")]
    MetadataTooLong,

    #[msg("Mint authority is not the expected program PDA")]
    UnexpectedMintAuthority,

    #[msg("Arithmetic overflow")]
    MathOverflow,
}
