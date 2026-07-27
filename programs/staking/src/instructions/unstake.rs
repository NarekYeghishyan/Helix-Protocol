//! Withdrawing principal.
//!
//! Early exit is refused outright rather than penalised. A penalty sounds
//! friendlier, but it creates a second reward-bearing balance that has to be
//! accounted for everywhere; refusing keeps the solvency invariant a single
//! subtraction. The lock is the product, not an inconvenience.

use anchor_lang::prelude::*;
use anchor_spl::token_interface::{self, Mint, TokenAccount, TokenInterface, TransferChecked};

use crate::constants::{POOL_SEED, POSITION_SEED, STAKE_VAULT_SEED, VAULT_AUTHORITY_SEED};
use crate::errors::StakingError;
use crate::events::Unstaked;
use crate::state::{Pool, Position};

#[derive(Accounts)]
pub struct Unstake<'info> {
    #[account(
        mut,
        seeds = [POOL_SEED, pool.stake_mint.as_ref(), pool.reward_mint.as_ref()],
        bump = pool.bump,
        has_one = stake_vault,
        has_one = stake_mint,
    )]
    pub pool: Box<Account<'info, Pool>>,

    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        mut,
        seeds = [
            POSITION_SEED,
            pool.key().as_ref(),
            owner.key().as_ref(),
            position.position_id.to_le_bytes().as_ref(),
        ],
        bump = position.bump,
        has_one = pool @ StakingError::PositionPoolMismatch,
        has_one = owner,
    )]
    pub position: Box<Account<'info, Position>>,

    pub stake_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        mut,
        seeds = [STAKE_VAULT_SEED, pool.key().as_ref()],
        bump,
        token::mint = stake_mint,
        token::token_program = token_program,
    )]
    pub stake_vault: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        token::mint = stake_mint,
        token::authority = owner,
        token::token_program = token_program,
    )]
    pub owner_token_account: Box<InterfaceAccount<'info, TokenAccount>>,

    /// CHECK: signs for the vault; identity fixed by seeds.
    #[account(
        seeds = [VAULT_AUTHORITY_SEED, pool.key().as_ref()],
        bump = pool.vault_authority_bump,
    )]
    pub vault_authority: UncheckedAccount<'info>,

    pub token_program: Interface<'info, TokenInterface>,
}

/// Withdraws `amount` of principal from an unlocked position.
///
/// Note that this is not gated on `pool.paused`. A pause that traps principal is
/// indistinguishable from a freeze — see `INVARIANTS.md` §6.4.
pub fn unstake(ctx: Context<Unstake>, amount: u64) -> Result<()> {
    require!(amount > 0, StakingError::ZeroAmount);

    let now = Clock::get()?.unix_timestamp;
    require!(
        ctx.accounts.position.is_unlocked(now),
        StakingError::PositionLocked
    );
    require!(
        amount <= ctx.accounts.position.amount,
        StakingError::InsufficientStake
    );

    // Book rewards at the current weight before the weight changes.
    ctx.accounts.pool.update_rewards(now)?;
    let reward_per_token = ctx.accounts.pool.reward_per_token;
    ctx.accounts.position.settle(reward_per_token)?;

    let position = &mut ctx.accounts.position;
    let old_weighted = position.weighted_amount;

    let remaining = position
        .amount
        .checked_sub(amount)
        .ok_or(StakingError::MathOverflow)?;

    // Recompute from the remaining principal rather than subtracting a weighted
    // share: recomputing keeps `weighted_amount == tier.apply_weight(amount)`
    // exactly true after any sequence of partial withdrawals, where repeated
    // proportional subtraction would accumulate truncation error.
    let new_weighted = position.tier.apply_weight(remaining)?;

    position.amount = remaining;
    position.weighted_amount = new_weighted;

    let pool_key = ctx.accounts.pool.key();
    let signer_seeds: &[&[&[u8]]] = &[&[
        VAULT_AUTHORITY_SEED,
        pool_key.as_ref(),
        &[ctx.accounts.pool.vault_authority_bump],
    ]];

    token_interface::transfer_checked(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            TransferChecked {
                from: ctx.accounts.stake_vault.to_account_info(),
                mint: ctx.accounts.stake_mint.to_account_info(),
                to: ctx.accounts.owner_token_account.to_account_info(),
                authority: ctx.accounts.vault_authority.to_account_info(),
            },
            signer_seeds,
        ),
        amount,
        ctx.accounts.stake_mint.decimals,
    )?;

    let weight_removed = old_weighted
        .checked_sub(new_weighted)
        .ok_or(StakingError::MathOverflow)?;

    let pool = &mut ctx.accounts.pool;
    pool.total_staked = pool
        .total_staked
        .checked_sub(amount)
        .ok_or(StakingError::MathOverflow)?;
    pool.total_weighted = pool
        .total_weighted
        .checked_sub(weight_removed)
        .ok_or(StakingError::MathOverflow)?;

    emit!(Unstaked {
        pool: pool_key,
        position: ctx.accounts.position.key(),
        owner: ctx.accounts.owner.key(),
        amount,
        remaining,
        timestamp: now,
    });

    Ok(())
}
