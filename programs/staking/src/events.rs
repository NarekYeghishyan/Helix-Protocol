use anchor_lang::prelude::*;

use crate::state::LockTier;

#[event]
pub struct PoolInitialized {
    pub pool: Pubkey,
    pub authority: Pubkey,
    pub stake_mint: Pubkey,
    pub reward_mint: Pubkey,
    pub timestamp: i64,
}

#[event]
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
pub struct Unstaked {
    pub pool: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub amount: u64,
    /// Principal left in the position; zero means it was closed.
    pub remaining: u64,
    pub timestamp: i64,
}

#[event]
pub struct RewardsClaimed {
    pub pool: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub amount: u64,
    pub timestamp: i64,
}

#[event]
pub struct RewardsFunded {
    pub pool: Pubkey,
    pub funder: Pubkey,
    pub amount_credited: u64,
    pub total_funded: u64,
    pub timestamp: i64,
}

#[event]
pub struct RewardRateChanged {
    pub pool: Pubkey,
    pub old_rate: u64,
    pub new_rate: u64,
    pub reward_period_end: i64,
    pub timestamp: i64,
}

#[event]
pub struct PoolPauseToggled {
    pub pool: Pubkey,
    pub paused: bool,
    pub timestamp: i64,
}

#[event]
pub struct AuthorityTransferProposed {
    pub pool: Pubkey,
    pub current_authority: Pubkey,
    pub pending_authority: Pubkey,
    pub timestamp: i64,
}

#[event]
pub struct AuthorityTransferAccepted {
    pub pool: Pubkey,
    pub previous_authority: Pubkey,
    pub new_authority: Pubkey,
    pub timestamp: i64,
}
