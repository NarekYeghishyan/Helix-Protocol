//! Events emitted on every state transition.
//!
//! These are the indexer's input ([`indexer/`](../../../indexer)). The dashboard
//! never polls account state — it reads what the programs reported at the moment
//! the change happened, which is the only way to reconstruct history rather than
//! just the present.
//!
//! Every event carries a `timestamp` taken from the on-chain clock rather than
//! relying on block time at ingestion, so replaying the log is deterministic.

use anchor_lang::prelude::*;

#[event]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenInitialized {
    pub config: Pubkey,
    pub mint: Pubkey,
    pub admin: Pubkey,
    pub decimals: u8,
    pub timestamp: i64,
}

#[event]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MinterRegistered {
    pub config: Pubkey,
    pub minter: Pubkey,
    pub authority: Pubkey,
    pub epoch_cap: u64,
    pub epoch_duration: i64,
    pub timestamp: i64,
}

#[event]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MinterUpdated {
    pub config: Pubkey,
    pub minter: Pubkey,
    pub epoch_cap: u64,
    pub enabled: bool,
    pub timestamp: i64,
}

#[event]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MinterRevoked {
    pub config: Pubkey,
    pub minter: Pubkey,
    pub authority: Pubkey,
    /// Retained so the indexer can close out the minter's lifetime issuance.
    pub total_minted: u64,
    pub timestamp: i64,
}

#[event]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokensMinted {
    pub config: Pubkey,
    pub minter: Pubkey,
    pub recipient: Pubkey,
    pub amount: u64,
    /// Supply after this issuance, so the indexer never has to reconcile a
    /// running total against a possibly-missed event.
    pub total_minted: u64,
    pub timestamp: i64,
}

#[event]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokensBurned {
    pub config: Pubkey,
    pub source: Pubkey,
    pub amount: u64,
    pub total_burned: u64,
    pub timestamp: i64,
}

#[event]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdminTransferProposed {
    pub config: Pubkey,
    pub current_admin: Pubkey,
    pub pending_admin: Pubkey,
    pub timestamp: i64,
}

/// A pending handover withdrawn before it was accepted.
///
/// Added because it was the one state transition in the protocol that changed an
/// account and emitted nothing. An observer would have seen
/// [`AdminTransferProposed`] and then silence, leaving any monitor tracking
/// "is a handover pending?" stuck on a false positive forever — during the
/// multi-step admin ceremony in `RUNBOOK.md`, which is precisely when someone is
/// watching.
#[event]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdminTransferCancelled {
    pub config: Pubkey,
    pub admin: Pubkey,
    /// The successor that will now not be taking over.
    pub cancelled_admin: Pubkey,
    pub timestamp: i64,
}

#[event]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdminTransferAccepted {
    pub config: Pubkey,
    pub previous_admin: Pubkey,
    pub new_admin: Pubkey,
    pub timestamp: i64,
}

#[event]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PauseToggled {
    pub config: Pubkey,
    pub paused: bool,
    pub admin: Pubkey,
    pub timestamp: i64,
}
