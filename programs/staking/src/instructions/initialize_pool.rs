//! Pool creation.
//!
//! Both vaults are PDAs owned by a single vault-authority PDA, so no key can
//! move principal or reward tokens. The authority recorded on the pool sets the
//! emission rate and funds rewards; it has no path to either vault's balance.

use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{POOL_SEED, REWARD_VAULT_SEED, STAKE_VAULT_SEED, VAULT_AUTHORITY_SEED};
use crate::events::PoolInitialized;
use crate::state::Pool;

#[derive(Accounts)]
pub struct InitializePool<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    /// Sets the reward rate and funds rewards. Deliberately not a signer here —
    /// pool creation grants it no power it could not be given later.
    /// CHECK: stored as configuration only.
    pub authority: UncheckedAccount<'info>,

    #[account(
        init,
        payer = payer,
        space = 8 + Pool::INIT_SPACE,
        seeds = [POOL_SEED, stake_mint.key().as_ref(), reward_mint.key().as_ref()],
        bump,
    )]
    pub pool: Account<'info, Pool>,

    /// CHECK: signs for both vaults. Never deserialised; identity fixed by seeds.
    #[account(
        seeds = [VAULT_AUTHORITY_SEED, pool.key().as_ref()],
        bump,
    )]
    pub vault_authority: UncheckedAccount<'info>,

    pub stake_mint: InterfaceAccount<'info, Mint>,
    pub reward_mint: InterfaceAccount<'info, Mint>,

    #[account(
        init,
        payer = payer,
        seeds = [STAKE_VAULT_SEED, pool.key().as_ref()],
        bump,
        token::mint = stake_mint,
        token::authority = vault_authority,
        token::token_program = token_program,
    )]
    pub stake_vault: InterfaceAccount<'info, TokenAccount>,

    #[account(
        init,
        payer = payer,
        seeds = [REWARD_VAULT_SEED, pool.key().as_ref()],
        bump,
        token::mint = reward_mint,
        token::authority = vault_authority,
        token::token_program = token_program,
    )]
    pub reward_vault: InterfaceAccount<'info, TokenAccount>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn initialize_pool(ctx: Context<InitializePool>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let pool = &mut ctx.accounts.pool;

    pool.authority = ctx.accounts.authority.key();
    pool.pending_authority = None;
    pool.stake_mint = ctx.accounts.stake_mint.key();
    pool.reward_mint = ctx.accounts.reward_mint.key();
    pool.stake_vault = ctx.accounts.stake_vault.key();
    pool.reward_vault = ctx.accounts.reward_vault.key();

    pool.total_staked = 0;
    pool.total_weighted = 0;

    // Emissions start switched off. `set_reward_rate` refuses any rate the
    // vault cannot fund, so a pool cannot promise rewards before it holds them.
    pool.reward_rate = 0;
    pool.reward_period_end = now;
    pool.reward_per_token = 0;
    pool.last_update_ts = now;

    pool.total_rewards_funded = 0;
    pool.total_rewards_paid = 0;
    pool.position_count = 0;
    pool.paused = false;

    pool.bump = ctx.bumps.pool;
    pool.vault_authority_bump = ctx.bumps.vault_authority;

    emit!(PoolInitialized {
        pool: pool.key(),
        authority: pool.authority,
        stake_mint: pool.stake_mint,
        reward_mint: pool.reward_mint,
        timestamp: now,
    });

    Ok(())
}
