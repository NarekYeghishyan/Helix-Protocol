#![allow(unexpected_cfgs)]
#![doc = include_str!("../README.md")]

use anchor_lang::prelude::*;

pub mod constants;
pub mod errors;
pub mod events;
pub mod instructions;
pub mod state;

use instructions::*;

declare_id!("B9HenpXUQzzGdT7mv93MQM8f6ytdPRKhJCbdx1CcBvdh");

#[program]
pub mod helix_treasury {
    use super::*;

    /// Creates a treasury and its PDA-owned vault, naming the governance
    /// executor that will be its sole spender.
    pub fn initialize_treasury(
        ctx: Context<InitializeTreasury>,
        epoch_spend_cap: u64,
        epoch_duration: i64,
    ) -> Result<()> {
        instructions::initialize::initialize_treasury(ctx, epoch_spend_cap, epoch_duration)
    }

    /// Moves tokens into the vault. Permissionless.
    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
        instructions::funds::deposit(ctx, amount)
    }

    /// Transfers out of the vault. Requires the governance executor's signature,
    /// respects the per-epoch cap, and cannot touch tokens committed to streams.
    pub fn spend(ctx: Context<Spend>, amount: u64) -> Result<()> {
        instructions::funds::spend(ctx, amount)
    }

    // ---------------------------------------------------------------- vesting

    /// Commits tokens to a linear vesting schedule for a beneficiary.
    pub fn create_stream(
        ctx: Context<CreateStream>,
        stream_id: u64,
        total_amount: u64,
        start_ts: i64,
        cliff_ts: i64,
        end_ts: i64,
    ) -> Result<()> {
        instructions::vesting::create_stream(
            ctx,
            stream_id,
            total_amount,
            start_ts,
            cliff_ts,
            end_ts,
        )
    }

    /// Withdraws everything vested and unclaimed. Beneficiary only.
    pub fn claim_stream(ctx: Context<ClaimStream>) -> Result<()> {
        instructions::vesting::claim_stream(ctx)
    }

    /// Stops future accrual. Already-vested tokens stay claimable.
    pub fn revoke_stream(ctx: Context<RevokeStream>) -> Result<()> {
        instructions::vesting::revoke_stream(ctx)
    }

    // ------------------------------------------------------------------ admin

    /// Adjusts the per-epoch spend cap.
    pub fn set_spend_cap(
        ctx: Context<GovernanceOnly>,
        new_cap: u64,
        epoch_duration: i64,
    ) -> Result<()> {
        instructions::initialize::set_spend_cap(ctx, new_cap, epoch_duration)
    }

    /// Hands spending rights to a different governance executor.
    pub fn set_governance_executor(
        ctx: Context<GovernanceOnly>,
        new_executor: Pubkey,
    ) -> Result<()> {
        instructions::initialize::set_governance_executor(ctx, new_executor)
    }
}
