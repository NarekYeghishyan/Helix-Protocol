//! Pool administration: funding, emission rate, pause, authority handover.

use anchor_lang::prelude::*;
use anchor_spl::token_interface::{self, Mint, TokenAccount, TokenInterface, TransferChecked};

use crate::constants::{MAX_REWARD_RATE, POOL_SEED, REWARD_VAULT_SEED};
use crate::errors::StakingError;
use crate::events::{
    AuthorityTransferAccepted, AuthorityTransferProposed, PoolPauseToggled, RewardRateChanged,
    RewardsFunded,
};
use crate::state::Pool;

#[derive(Accounts)]
pub struct FundRewards<'info> {
    #[account(
        mut,
        seeds = [POOL_SEED, pool.stake_mint.as_ref(), pool.reward_mint.as_ref()],
        bump = pool.bump,
        has_one = reward_vault,
        has_one = reward_mint,
    )]
    pub pool: Account<'info, Pool>,

    /// Anyone may top up the reward vault. Funding the pool can only benefit
    /// stakers, so there is no reason to restrict it to the authority.
    #[account(mut)]
    pub funder: Signer<'info>,

    pub reward_mint: InterfaceAccount<'info, Mint>,

    #[account(
        mut,
        token::mint = reward_mint,
        token::authority = funder,
        token::token_program = token_program,
    )]
    pub funder_token_account: InterfaceAccount<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [REWARD_VAULT_SEED, pool.key().as_ref()],
        bump,
        token::mint = reward_mint,
        token::token_program = token_program,
    )]
    pub reward_vault: InterfaceAccount<'info, TokenAccount>,

    pub token_program: Interface<'info, TokenInterface>,
}

/// Transfers reward tokens into the vault.
///
/// As in `stake`, the credited figure is the observed vault delta, so a
/// fee-bearing reward mint cannot cause the pool to believe it holds more than
/// it does.
pub fn fund_rewards(ctx: Context<FundRewards>, amount: u64) -> Result<()> {
    require!(amount > 0, StakingError::ZeroAmount);

    let now = Clock::get()?.unix_timestamp;
    let balance_before = ctx.accounts.reward_vault.amount;

    token_interface::transfer_checked(
        CpiContext::new(
            ctx.accounts.token_program.key(),
            TransferChecked {
                from: ctx.accounts.funder_token_account.to_account_info(),
                mint: ctx.accounts.reward_mint.to_account_info(),
                to: ctx.accounts.reward_vault.to_account_info(),
                authority: ctx.accounts.funder.to_account_info(),
            },
        ),
        amount,
        ctx.accounts.reward_mint.decimals,
    )?;

    ctx.accounts.reward_vault.reload()?;
    let credited = ctx
        .accounts
        .reward_vault
        .amount
        .checked_sub(balance_before)
        .ok_or(StakingError::VaultBalanceMismatch)?;
    require!(credited > 0, StakingError::ZeroAfterFees);

    let pool = &mut ctx.accounts.pool;
    pool.total_rewards_funded = pool
        .total_rewards_funded
        .checked_add(credited)
        .ok_or(StakingError::MathOverflow)?;

    emit!(RewardsFunded {
        pool: pool.key(),
        funder: ctx.accounts.funder.key(),
        amount_credited: credited,
        total_funded: pool.total_rewards_funded,
        timestamp: now,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct SetRewardRate<'info> {
    #[account(
        mut,
        seeds = [POOL_SEED, pool.stake_mint.as_ref(), pool.reward_mint.as_ref()],
        bump = pool.bump,
        has_one = authority @ StakingError::NotAuthority,
        has_one = reward_vault,
    )]
    pub pool: Account<'info, Pool>,

    pub authority: Signer<'info>,

    #[account(
        seeds = [REWARD_VAULT_SEED, pool.key().as_ref()],
        bump,
    )]
    pub reward_vault: InterfaceAccount<'info, TokenAccount>,
}

/// Sets the emission rate and the timestamp emissions run until.
///
/// Refuses any combination the vault cannot actually pay for. A pool that can
/// promise more than it holds is insolvent from that moment, but the failure
/// only surfaces later, as a confusing transfer error for whoever happens to
/// claim last (`INVARIANTS.md` §1.2). Checking here turns a silent future
/// failure into an immediate, obvious one.
pub fn set_reward_rate(
    ctx: Context<SetRewardRate>,
    new_rate: u64,
    reward_period_end: i64,
) -> Result<()> {
    require!(new_rate <= MAX_REWARD_RATE, StakingError::RewardRateTooHigh);

    let now = Clock::get()?.unix_timestamp;
    require!(reward_period_end >= now, StakingError::InvalidRewardPeriod);

    // Settle everything owed under the old rate before changing it, so the
    // change applies only from this moment forward.
    ctx.accounts.pool.update_rewards(now)?;

    let pool = &mut ctx.accounts.pool;
    let old_rate = pool.reward_rate;
    pool.reward_period_end = reward_period_end;

    // Already-accrued rewards that nobody has claimed yet, plus everything the
    // new rate commits to, must both fit inside the vault.
    let outstanding = pool.unpaid_liability()?;
    let committed = pool.emission_for(new_rate, now)?;
    let required = outstanding
        .checked_add(committed)
        .ok_or(StakingError::MathOverflow)?;

    require!(
        required <= ctx.accounts.reward_vault.amount,
        StakingError::InsufficientRewardFunding
    );

    pool.reward_rate = new_rate;

    emit!(RewardRateChanged {
        pool: pool.key(),
        old_rate,
        new_rate,
        reward_period_end,
        timestamp: now,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct PoolAuthorityOnly<'info> {
    #[account(
        mut,
        seeds = [POOL_SEED, pool.stake_mint.as_ref(), pool.reward_mint.as_ref()],
        bump = pool.bump,
        has_one = authority @ StakingError::NotAuthority,
    )]
    pub pool: Account<'info, Pool>,

    pub authority: Signer<'info>,
}

/// Blocks new deposits.
///
/// Unstaking and claiming stay live by design (`INVARIANTS.md` §6.4) — the pause
/// exists to stop the pool taking on new exposure during an incident, not to
/// trap the people already in it.
pub fn set_paused(ctx: Context<PoolAuthorityOnly>, paused: bool) -> Result<()> {
    let pool = &mut ctx.accounts.pool;
    pool.paused = paused;

    emit!(PoolPauseToggled {
        pool: pool.key(),
        paused,
        timestamp: Clock::get()?.unix_timestamp,
    });

    Ok(())
}

/// Step one of the two-step authority handover.
pub fn propose_authority(ctx: Context<PoolAuthorityOnly>, new_authority: Pubkey) -> Result<()> {
    let pool = &mut ctx.accounts.pool;
    pool.pending_authority = Some(new_authority);

    emit!(AuthorityTransferProposed {
        pool: pool.key(),
        current_authority: pool.authority,
        pending_authority: new_authority,
        timestamp: Clock::get()?.unix_timestamp,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct AcceptAuthority<'info> {
    #[account(
        mut,
        seeds = [POOL_SEED, pool.stake_mint.as_ref(), pool.reward_mint.as_ref()],
        bump = pool.bump,
    )]
    pub pool: Account<'info, Pool>,

    /// The proposed successor, proving key custody by signing.
    pub new_authority: Signer<'info>,
}

/// Step two. Only the proposed successor can complete the handover.
pub fn accept_authority(ctx: Context<AcceptAuthority>) -> Result<()> {
    let pool = &mut ctx.accounts.pool;

    let pending = pool
        .pending_authority
        .ok_or(StakingError::NoPendingAuthority)?;
    require_keys_eq!(
        pending,
        ctx.accounts.new_authority.key(),
        StakingError::NotPendingAuthority
    );

    let previous_authority = pool.authority;
    pool.authority = pending;
    pool.pending_authority = None;

    emit!(AuthorityTransferAccepted {
        pool: pool.key(),
        previous_authority,
        new_authority: pool.authority,
        timestamp: Clock::get()?.unix_timestamp,
    });

    Ok(())
}
