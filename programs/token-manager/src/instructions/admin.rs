//! Admin lifecycle: two-step transfer, and the pause switch.

use anchor_lang::prelude::*;

use crate::constants::CONFIG_SEED;
use crate::errors::TokenManagerError;
use crate::events::{
    AdminTransferAccepted, AdminTransferCancelled, AdminTransferProposed, PauseToggled,
};
use crate::state::TokenConfig;

#[derive(Accounts)]
pub struct AdminOnly<'info> {
    #[account(
        mut,
        seeds = [CONFIG_SEED, config.mint.as_ref()],
        bump = config.bump,
        has_one = admin @ TokenManagerError::NotAdmin,
    )]
    pub config: Account<'info, TokenConfig>,

    pub admin: Signer<'info>,
}

/// Step one of an admin handover. Records the intended successor without
/// granting them anything.
pub fn propose_admin(ctx: Context<AdminOnly>, new_admin: Pubkey) -> Result<()> {
    let config = &mut ctx.accounts.config;
    require_keys_neq!(new_admin, config.admin, TokenManagerError::AdminUnchanged);

    config.pending_admin = Some(new_admin);

    emit!(AdminTransferProposed {
        config: config.key(),
        current_admin: config.admin,
        pending_admin: new_admin,
        timestamp: Clock::get()?.unix_timestamp,
    });

    Ok(())
}

/// Cancels a pending handover. Cheap insurance against a proposal made in error.
pub fn cancel_admin_transfer(ctx: Context<AdminOnly>) -> Result<()> {
    let config = &mut ctx.accounts.config;
    let cancelled_admin = config
        .pending_admin
        .ok_or(TokenManagerError::NoPendingAdmin)?;

    config.pending_admin = None;

    emit!(AdminTransferCancelled {
        config: config.key(),
        admin: config.admin,
        cancelled_admin,
        timestamp: Clock::get()?.unix_timestamp,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct AcceptAdmin<'info> {
    #[account(
        mut,
        seeds = [CONFIG_SEED, config.mint.as_ref()],
        bump = config.bump,
    )]
    pub config: Account<'info, TokenConfig>,

    /// The proposed successor, proving key custody by signing. This signature is
    /// the entire point of the two-step flow: an address that cannot sign cannot
    /// become admin, so a typo'd or otherwise unusable address is caught here
    /// instead of permanently orphaning the role.
    pub new_admin: Signer<'info>,
}

/// Step two. Only the proposed successor can complete the transfer.
pub fn accept_admin(ctx: Context<AcceptAdmin>) -> Result<()> {
    let config = &mut ctx.accounts.config;

    let pending = config
        .pending_admin
        .ok_or(TokenManagerError::NoPendingAdmin)?;
    require_keys_eq!(
        pending,
        ctx.accounts.new_admin.key(),
        TokenManagerError::NotPendingAdmin
    );

    let previous_admin = config.admin;
    config.admin = pending;
    config.pending_admin = None;

    emit!(AdminTransferAccepted {
        config: config.key(),
        previous_admin,
        new_admin: config.admin,
        timestamp: Clock::get()?.unix_timestamp,
    });

    Ok(())
}

/// Halts new issuance.
///
/// Note what this deliberately does *not* stop: burning. A pause that blocks the
/// exit path is a freeze, and a freeze is indistinguishable from a rug from the
/// holder's side. See `INVARIANTS.md` §6.4 for the equivalent limit on staking.
pub fn set_paused(ctx: Context<AdminOnly>, paused: bool) -> Result<()> {
    let config = &mut ctx.accounts.config;
    config.paused = paused;

    emit!(PauseToggled {
        config: config.key(),
        paused,
        admin: config.admin,
        timestamp: Clock::get()?.unix_timestamp,
    });

    Ok(())
}
