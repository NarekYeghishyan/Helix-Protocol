#![allow(unexpected_cfgs)]
#![doc = include_str!("../README.md")]

use anchor_lang::prelude::*;

pub mod constants;
pub mod errors;
pub mod events;
pub mod instructions;
pub mod state;

use instructions::*;
use state::LockTier;

declare_id!("9RuZJZpgCwbiF9JRAsyR8cqDhFSaFYus1mzobKzEZzP3");

#[program]
pub mod helix_staking {
    use super::*;

    /// Creates a pool for a (stake mint, reward mint) pair, with both vaults
    /// owned by a PDA. Emissions start at zero.
    pub fn initialize_pool(ctx: Context<InitializePool>) -> Result<()> {
        instructions::initialize_pool::initialize_pool(ctx)
    }

    // ----------------------------------------------------------------- staker

    /// Opens a position of `amount` under `tier`. `position_id` must equal the
    /// pool's current `position_count`.
    pub fn stake(ctx: Context<Stake>, position_id: u64, amount: u64, tier: LockTier) -> Result<()> {
        instructions::stake::stake(ctx, position_id, amount, tier)
    }

    /// Withdraws principal from an unlocked position. Not blocked by pause.
    pub fn unstake(ctx: Context<Unstake>, amount: u64) -> Result<()> {
        instructions::unstake::unstake(ctx, amount)
    }

    /// Withdraws accrued rewards. Available regardless of lock or pause state.
    pub fn claim(ctx: Context<Claim>) -> Result<()> {
        instructions::claim::claim(ctx)
    }

    /// Reclaims the rent of a position that holds nothing. Refused while any
    /// principal, weight or unclaimed reward remains.
    pub fn close_position(ctx: Context<ClosePosition>) -> Result<()> {
        instructions::close_position::close_position(ctx)
    }

    // ------------------------------------------------------------------ admin

    /// Tops up the reward vault. Permissionless.
    pub fn fund_rewards(ctx: Context<FundRewards>, amount: u64) -> Result<()> {
        instructions::admin::fund_rewards(ctx, amount)
    }

    /// Sets emission rate and period end. Refuses a rate the vault cannot fund.
    pub fn set_reward_rate(
        ctx: Context<SetRewardRate>,
        new_rate: u64,
        reward_period_end: i64,
    ) -> Result<()> {
        instructions::admin::set_reward_rate(ctx, new_rate, reward_period_end)
    }

    /// Blocks new deposits. Unstaking and claiming stay live.
    pub fn set_paused(ctx: Context<PoolAuthorityOnly>, paused: bool) -> Result<()> {
        instructions::admin::set_paused(ctx, paused)
    }

    /// Step one of a two-step authority handover.
    pub fn propose_authority(ctx: Context<PoolAuthorityOnly>, new_authority: Pubkey) -> Result<()> {
        instructions::admin::propose_authority(ctx, new_authority)
    }

    /// Step two. Callable only by the proposed successor.
    pub fn accept_authority(ctx: Context<AcceptAuthority>) -> Result<()> {
        instructions::admin::accept_authority(ctx)
    }
}
