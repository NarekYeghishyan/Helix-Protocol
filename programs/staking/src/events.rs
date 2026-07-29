//! Events are this program's public data interface: consumers reconstruct state
//! by decoding these from transaction logs rather than by reading accounts. They
//! derive `Clone`/`Debug`/`PartialEq` so a consumer can hold, compare and test
//! against them — `#[event]` alone provides only the Borsh pair. See
//! [`indexer/`](../../../../indexer).

use anchor_lang::prelude::*;

use crate::state::LockTier;

#[event]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolInitialized {
    pub pool: Pubkey,
    pub authority: Pubkey,
    pub stake_mint: Pubkey,
    pub reward_mint: Pubkey,
    pub timestamp: i64,
}

#[event]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Staked {
    pub pool: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub position_id: u64,
    /// What the caller sent.
    pub amount_sent: u64,
    /// What the vault actually received. These differ when the stake mint
    /// carries a Token-2022 transfer fee, and the credited figure is the one
    /// the position is built from.
    pub amount_credited: u64,
    pub weighted_amount: u64,
    pub tier: LockTier,
    pub lock_end: i64,
    pub timestamp: i64,
}

#[event]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unstaked {
    pub pool: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub amount: u64,
    /// Principal left in the position. Zero means it is fully exited, not that
    /// the account is gone — reclaiming the rent is a separate, optional step
    /// that emits [`PositionClosed`].
    pub remaining: u64,
    /// Vote weight left in the position, so a consumer never has to recompute
    /// it from `remaining` and the tier.
    ///
    /// Added when the indexer was built. Without it, reconstructing
    /// `pool.total_weighted` from the event stream means re-running
    /// `LockTier::apply_weight` off chain — a second implementation of the
    /// weight table that agrees with the program until the day the table
    /// changes, and then disagrees silently. An event that cannot be folded into
    /// state without recomputation is an incomplete event.
    pub weighted_amount: u64,
    pub timestamp: i64,
}

/// A fully exited position's account was deallocated and its rent returned.
///
/// Carries `position_id` because the account it describes no longer exists —
/// a consumer that wanted the id would otherwise have to have retained the
/// `Staked` event that opened it. This is the same rule `Unstaked` produced:
/// an event that cannot be folded into state without going elsewhere for a
/// field is an incomplete event.
///
/// Emphatically **not** a decrement of `pool.position_count`, which counts
/// positions ever opened — see [`crate::instructions::close_position`].
#[event]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PositionClosed {
    pub pool: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub position_id: u64,
    pub timestamp: i64,
}

#[event]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RewardsClaimed {
    pub pool: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub amount: u64,
    pub timestamp: i64,
}

#[event]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RewardsFunded {
    pub pool: Pubkey,
    pub funder: Pubkey,
    pub amount_credited: u64,
    pub total_funded: u64,
    pub timestamp: i64,
}

#[event]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RewardRateChanged {
    pub pool: Pubkey,
    pub old_rate: u64,
    pub new_rate: u64,
    pub reward_period_end: i64,
    pub timestamp: i64,
}

#[event]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolPauseToggled {
    pub pool: Pubkey,
    pub paused: bool,
    pub timestamp: i64,
}

#[event]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorityTransferProposed {
    pub pool: Pubkey,
    pub current_authority: Pubkey,
    pub pending_authority: Pubkey,
    pub timestamp: i64,
}

#[event]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorityTransferAccepted {
    pub pool: Pubkey,
    pub previous_authority: Pubkey,
    pub new_authority: Pubkey,
    pub timestamp: i64,
}
