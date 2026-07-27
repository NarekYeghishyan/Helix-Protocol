//! Treasury creation and configuration.

use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{
    MAX_EPOCH_DURATION, MIN_EPOCH_DURATION, TREASURY_SEED, VAULT_AUTHORITY_SEED, VAULT_SEED,
};
use crate::errors::TreasuryError;
use crate::events::{GovernanceExecutorChanged, SpendCapChanged, TreasuryInitialized};
use crate::state::Treasury;

#[derive(Accounts)]
pub struct InitializeTreasury<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    /// The governance execution PDA that will be the sole spender.
    /// CHECK: stored as configuration; its authority is exercised only by
    /// signing later instructions, which is checked at that point.
    pub governance_executor: UncheckedAccount<'info>,

    #[account(
        init,
        payer = payer,
        space = 8 + Treasury::INIT_SPACE,
        seeds = [TREASURY_SEED, mint.key().as_ref()],
        bump,
    )]
    pub treasury: Account<'info, Treasury>,

    /// CHECK: signs for the vault; identity fixed by seeds.
    #[account(
        seeds = [VAULT_AUTHORITY_SEED, treasury.key().as_ref()],
        bump,
    )]
    pub vault_authority: UncheckedAccount<'info>,

    pub mint: InterfaceAccount<'info, Mint>,

    #[account(
        init,
        payer = payer,
        seeds = [VAULT_SEED, treasury.key().as_ref()],
        bump,
        token::mint = mint,
        token::authority = vault_authority,
        token::token_program = token_program,
    )]
    pub vault: InterfaceAccount<'info, TokenAccount>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn initialize_treasury(
    ctx: Context<InitializeTreasury>,
    epoch_spend_cap: u64,
    epoch_duration: i64,
) -> Result<()> {
    require!(
        (MIN_EPOCH_DURATION..=MAX_EPOCH_DURATION).contains(&epoch_duration),
        TreasuryError::InvalidEpochDuration
    );

    let now = Clock::get()?.unix_timestamp;
    let treasury = &mut ctx.accounts.treasury;

    treasury.governance_executor = ctx.accounts.governance_executor.key();
    treasury.mint = ctx.accounts.mint.key();
    treasury.vault = ctx.accounts.vault.key();

    treasury.total_deposited = 0;
    treasury.total_spent = 0;
    treasury.committed_to_streams = 0;

    treasury.epoch_duration = epoch_duration;
    treasury.epoch_spend_cap = epoch_spend_cap;
    treasury.spent_this_epoch = 0;
    treasury.stream_count = 0;

    treasury.bump = ctx.bumps.treasury;
    treasury.vault_authority_bump = ctx.bumps.vault_authority;

    // Anchor the window to the current epoch so a new treasury does not inherit
    // a spent balance from epoch zero.
    treasury.current_epoch = treasury.epoch_at(now)?;

    emit!(TreasuryInitialized {
        treasury: treasury.key(),
        governance_executor: treasury.governance_executor,
        mint: treasury.mint,
        epoch_spend_cap,
        epoch_duration,
        timestamp: now,
    });

    Ok(())
}

/// Accounts for any instruction only the governance executor may call.
#[derive(Accounts)]
pub struct GovernanceOnly<'info> {
    #[account(
        mut,
        seeds = [TREASURY_SEED, treasury.mint.as_ref()],
        bump = treasury.bump,
        has_one = governance_executor @ TreasuryError::NotGovernanceExecutor,
    )]
    pub treasury: Account<'info, Treasury>,

    /// The governance program's execution PDA, signing through a CPI inside
    /// `execute_proposal`. No other signer satisfies this constraint, which is
    /// what makes every treasury change an act of governance.
    pub governance_executor: Signer<'info>,
}

/// Adjusts the per-epoch spend cap.
pub fn set_spend_cap(
    ctx: Context<GovernanceOnly>,
    new_cap: u64,
    epoch_duration: i64,
) -> Result<()> {
    require!(
        (MIN_EPOCH_DURATION..=MAX_EPOCH_DURATION).contains(&epoch_duration),
        TreasuryError::InvalidEpochDuration
    );

    let treasury = &mut ctx.accounts.treasury;
    let old_cap = treasury.epoch_spend_cap;
    treasury.epoch_spend_cap = new_cap;
    treasury.epoch_duration = epoch_duration;

    emit!(SpendCapChanged {
        treasury: treasury.key(),
        old_cap,
        new_cap,
        epoch_duration,
        timestamp: Clock::get()?.unix_timestamp,
    });

    Ok(())
}

/// Hands spending rights to a different governance executor.
///
/// Only the current executor can do this, so migrating to a new governance
/// program is itself something the existing governance has to vote for. There is
/// deliberately no admin escape hatch here — an escape hatch would make every
/// other guarantee in this program conditional on whoever holds it.
pub fn set_governance_executor(ctx: Context<GovernanceOnly>, new_executor: Pubkey) -> Result<()> {
    let treasury = &mut ctx.accounts.treasury;
    let previous_executor = treasury.governance_executor;
    treasury.governance_executor = new_executor;

    emit!(GovernanceExecutorChanged {
        treasury: treasury.key(),
        previous_executor,
        new_executor,
        timestamp: Clock::get()?.unix_timestamp,
    });

    Ok(())
}
