//! The minter registry.
//!
//! The mint authority lives in a PDA, so nothing can mint without this program
//! signing for it — and this program only signs on behalf of an authority
//! recorded here, within that authority's per-epoch cap. The registry is
//! therefore the whole of the issuance policy.

use anchor_lang::prelude::*;

use crate::constants::{CONFIG_SEED, MAX_MINTERS, MINTER_SEED, MIN_EPOCH_DURATION};
use crate::errors::TokenManagerError;
use crate::events::{MinterRegistered, MinterRevoked, MinterUpdated};
use crate::state::{Minter, TokenConfig};

#[derive(Accounts)]
#[instruction(authority: Pubkey)]
pub struct RegisterMinter<'info> {
    #[account(
        mut,
        seeds = [CONFIG_SEED, config.mint.as_ref()],
        bump = config.bump,
        has_one = admin @ TokenManagerError::NotAdmin,
    )]
    pub config: Account<'info, TokenConfig>,

    pub admin: Signer<'info>,

    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(
        init,
        payer = payer,
        space = 8 + Minter::INIT_SPACE,
        seeds = [MINTER_SEED, config.key().as_ref(), authority.as_ref()],
        bump,
    )]
    pub minter: Account<'info, Minter>,

    pub system_program: Program<'info, System>,
}

pub fn register_minter(
    ctx: Context<RegisterMinter>,
    authority: Pubkey,
    epoch_cap: u64,
    epoch_duration: i64,
) -> Result<()> {
    require!(
        epoch_duration >= MIN_EPOCH_DURATION,
        TokenManagerError::InvalidEpochDuration
    );

    let config = &mut ctx.accounts.config;
    require!(
        config.minter_count < MAX_MINTERS,
        TokenManagerError::MinterRegistryFull
    );

    let now = Clock::get()?.unix_timestamp;

    let minter = &mut ctx.accounts.minter;
    minter.config = config.key();
    minter.authority = authority;
    minter.epoch_cap = epoch_cap;
    minter.minted_this_epoch = 0;
    minter.epoch_duration = epoch_duration;
    minter.total_minted = 0;
    minter.enabled = true;
    minter.bump = ctx.bumps.minter;
    // Start the window at the current epoch so a freshly registered minter does
    // not inherit an allowance from epoch zero.
    minter.current_epoch = minter.epoch_at(now)?;

    config.minter_count = config
        .minter_count
        .checked_add(1)
        .ok_or(TokenManagerError::MathOverflow)?;

    emit!(MinterRegistered {
        config: config.key(),
        minter: minter.key(),
        authority,
        epoch_cap,
        epoch_duration,
        timestamp: now,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct ModifyMinter<'info> {
    #[account(
        seeds = [CONFIG_SEED, config.mint.as_ref()],
        bump = config.bump,
        has_one = admin @ TokenManagerError::NotAdmin,
    )]
    pub config: Account<'info, TokenConfig>,

    pub admin: Signer<'info>,

    #[account(
        mut,
        seeds = [MINTER_SEED, config.key().as_ref(), minter.authority.as_ref()],
        bump = minter.bump,
        has_one = config,
    )]
    pub minter: Account<'info, Minter>,
}

/// Adjusts a minter's cap or enables/disables it.
///
/// Lowering the cap below what has already been issued this epoch is permitted
/// and simply leaves no headroom until the window rolls — `Minter::accrue`
/// compares against the cap, so the already-issued amount is never clawed back.
pub fn update_minter(ctx: Context<ModifyMinter>, epoch_cap: u64, enabled: bool) -> Result<()> {
    let minter = &mut ctx.accounts.minter;
    minter.epoch_cap = epoch_cap;
    minter.enabled = enabled;

    emit!(MinterUpdated {
        config: ctx.accounts.config.key(),
        minter: minter.key(),
        epoch_cap,
        enabled,
        timestamp: Clock::get()?.unix_timestamp,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct RevokeMinter<'info> {
    #[account(
        mut,
        seeds = [CONFIG_SEED, config.mint.as_ref()],
        bump = config.bump,
        has_one = admin @ TokenManagerError::NotAdmin,
    )]
    pub config: Account<'info, TokenConfig>,

    pub admin: Signer<'info>,

    #[account(
        mut,
        seeds = [MINTER_SEED, config.key().as_ref(), minter.authority.as_ref()],
        bump = minter.bump,
        has_one = config,
    )]
    pub minter: Account<'info, Minter>,
}

/// Permanently disables a minter.
///
/// This disables rather than closing the account. Closing would refund rent and
/// tidy up, but it would also erase `total_minted` — and the record of how much
/// a since-revoked minter issued is exactly what someone auditing the supply
/// later needs. Rent is a small price for an immutable audit trail.
pub fn revoke_minter(ctx: Context<RevokeMinter>) -> Result<()> {
    let minter = &mut ctx.accounts.minter;
    minter.enabled = false;
    minter.epoch_cap = 0;

    let config = &mut ctx.accounts.config;
    config.minter_count = config.minter_count.saturating_sub(1);

    emit!(MinterRevoked {
        config: config.key(),
        minter: minter.key(),
        authority: minter.authority,
        total_minted: minter.total_minted,
        timestamp: Clock::get()?.unix_timestamp,
    });

    Ok(())
}
