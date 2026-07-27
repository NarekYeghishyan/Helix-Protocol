//! Claiming accrued rewards.

use anchor_lang::prelude::*;
use anchor_spl::token_interface::{self, Mint, TokenAccount, TokenInterface, TransferChecked};

use crate::constants::{POOL_SEED, POSITION_SEED, REWARD_VAULT_SEED, VAULT_AUTHORITY_SEED};
use crate::errors::StakingError;
use crate::events::RewardsClaimed;
use crate::state::{Pool, Position};

#[derive(Accounts)]
pub struct Claim<'info> {
    #[account(
        mut,
        seeds = [POOL_SEED, pool.stake_mint.as_ref(), pool.reward_mint.as_ref()],
        bump = pool.bump,
        has_one = reward_vault,
        has_one = reward_mint,
    )]
    pub pool: Box<Account<'info, Pool>>,

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

    pub reward_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        mut,
        seeds = [REWARD_VAULT_SEED, pool.key().as_ref()],
        bump,
        token::mint = reward_mint,
        token::token_program = token_program,
    )]
    pub reward_vault: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        token::mint = reward_mint,
        token::authority = owner,
        token::token_program = token_program,
    )]
    pub owner_reward_account: Box<InterfaceAccount<'info, TokenAccount>>,

    /// CHECK: signs for the vault; identity fixed by seeds.
    #[account(
        seeds = [VAULT_AUTHORITY_SEED, pool.key().as_ref()],
        bump = pool.vault_authority_bump,
    )]
    pub vault_authority: UncheckedAccount<'info>,

    pub token_program: Interface<'info, TokenInterface>,
}

/// Withdraws everything the position has accrued.
///
/// Deliberately available regardless of lock state and regardless of the pause
/// flag (`INVARIANTS.md` §6.1). Locking principal is the bargain the staker
/// agreed to; locking the yield on top of it is not.
pub fn claim(ctx: Context<Claim>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;

    ctx.accounts.pool.update_rewards(now)?;
    let reward_per_token = ctx.accounts.pool.reward_per_token;
    ctx.accounts.position.settle(reward_per_token)?;

    let amount = ctx.accounts.position.pending_rewards;
    require!(amount > 0, StakingError::NothingToClaim);

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
                from: ctx.accounts.reward_vault.to_account_info(),
                mint: ctx.accounts.reward_mint.to_account_info(),
                to: ctx.accounts.owner_reward_account.to_account_info(),
                authority: ctx.accounts.vault_authority.to_account_info(),
            },
            signer_seeds,
        ),
        amount,
        ctx.accounts.reward_mint.decimals,
    )?;

    // Zero the credit only after the transfer has succeeded.
    ctx.accounts.position.pending_rewards = 0;

    let pool = &mut ctx.accounts.pool;
    pool.total_rewards_paid = pool
        .total_rewards_paid
        .checked_add(amount)
        .ok_or(StakingError::MathOverflow)?;

    emit!(RewardsClaimed {
        pool: pool_key,
        position: ctx.accounts.position.key(),
        owner: ctx.accounts.owner.key(),
        amount,
        timestamp: now,
    });

    Ok(())
}
