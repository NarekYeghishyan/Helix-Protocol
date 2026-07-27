#![allow(unexpected_cfgs)]
#![doc = include_str!("../README.md")]

use anchor_lang::prelude::*;

pub mod constants;
pub mod errors;
pub mod events;
pub mod instructions;
pub mod state;

use instructions::*;

declare_id!("5RU35Eni3MxkuSc9Zv5xm8LLd2QX85XdbYjRUaLkFRFr");

#[program]
pub mod helix_token_manager {
    use super::*;

    /// Creates the HLX mint with a PDA mint authority and the config that
    /// governs it. After this call no key can mint.
    pub fn initialize_token(
        ctx: Context<InitializeToken>,
        args: InitializeTokenArgs,
    ) -> Result<()> {
        instructions::initialize_token::initialize_token(ctx, args)
    }

    // ---------------------------------------------------------------- registry

    /// Grants `authority` the right to mint, up to `epoch_cap` per epoch.
    pub fn register_minter(
        ctx: Context<RegisterMinter>,
        authority: Pubkey,
        epoch_cap: u64,
        epoch_duration: i64,
    ) -> Result<()> {
        instructions::minter_registry::register_minter(ctx, authority, epoch_cap, epoch_duration)
    }

    /// Adjusts a minter's cap, or enables/disables it.
    pub fn update_minter(ctx: Context<ModifyMinter>, epoch_cap: u64, enabled: bool) -> Result<()> {
        instructions::minter_registry::update_minter(ctx, epoch_cap, enabled)
    }

    /// Permanently disables a minter, retaining its issuance history.
    pub fn revoke_minter(ctx: Context<RevokeMinter>) -> Result<()> {
        instructions::minter_registry::revoke_minter(ctx)
    }

    // ---------------------------------------------------------------- issuance

    /// Mints `amount` to `recipient`. Requires a registered, enabled minter with
    /// headroom under its per-epoch cap.
    pub fn mint_tokens(ctx: Context<MintTokens>, amount: u64) -> Result<()> {
        instructions::issuance::mint_to(ctx, amount)
    }

    /// Burns `amount` from the caller's own token account.
    pub fn burn_tokens(ctx: Context<BurnTokens>, amount: u64) -> Result<()> {
        instructions::issuance::burn(ctx, amount)
    }

    // ------------------------------------------------------------------- admin

    /// Step one of a two-step admin handover.
    pub fn propose_admin(ctx: Context<AdminOnly>, new_admin: Pubkey) -> Result<()> {
        instructions::admin::propose_admin(ctx, new_admin)
    }

    /// Withdraws a pending handover.
    pub fn cancel_admin_transfer(ctx: Context<AdminOnly>) -> Result<()> {
        instructions::admin::cancel_admin_transfer(ctx)
    }

    /// Step two. Callable only by the proposed successor.
    pub fn accept_admin(ctx: Context<AcceptAdmin>) -> Result<()> {
        instructions::admin::accept_admin(ctx)
    }

    /// Halts new issuance. Does not block burning.
    pub fn set_paused(ctx: Context<AdminOnly>, paused: bool) -> Result<()> {
        instructions::admin::set_paused(ctx, paused)
    }
}
