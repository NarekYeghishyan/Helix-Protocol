//! Creates the HLX mint and hands its authority to a PDA.
//!
//! After this instruction there is no key anywhere that can mint HLX. The only
//! path to new supply is [`crate::instructions::issuance::mint_to`], gated by
//! the minter registry.

use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenInterface};

use crate::constants::{
    CONFIG_SEED, MAX_NAME_LEN, MAX_SYMBOL_LEN, MAX_URI_LEN, MINT_AUTHORITY_SEED,
};
use crate::errors::TokenManagerError;
use crate::events::TokenInitialized;
use crate::state::TokenConfig;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct InitializeTokenArgs {
    pub decimals: u8,
    pub name: String,
    pub symbol: String,
    pub uri: String,
}

impl InitializeTokenArgs {
    /// Metadata is bounded so account sizing stays a function of constants
    /// rather than of caller-controlled input.
    pub fn validate(&self) -> Result<()> {
        require!(
            self.name.len() <= MAX_NAME_LEN
                && self.symbol.len() <= MAX_SYMBOL_LEN
                && self.uri.len() <= MAX_URI_LEN,
            TokenManagerError::MetadataTooLong
        );
        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(args: InitializeTokenArgs)]
pub struct InitializeToken<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    /// The initial admin. Only registers minters and toggles the pause — it
    /// never gains minting rights, so this does not need to be a signer.
    /// CHECK: stored as configuration, never used as an authority here.
    pub admin: UncheckedAccount<'info>,

    #[account(
        init,
        payer = payer,
        space = 8 + TokenConfig::INIT_SPACE,
        seeds = [CONFIG_SEED, mint.key().as_ref()],
        bump,
    )]
    pub config: Account<'info, TokenConfig>,

    /// CHECK: holds mint and freeze authority. Never deserialised; it only
    /// signs, and its identity is fixed by the seeds constraint.
    #[account(
        seeds = [MINT_AUTHORITY_SEED, config.key().as_ref()],
        bump,
    )]
    pub mint_authority: UncheckedAccount<'info>,

    /// A fresh Token-2022 mint. Freeze authority is set to the same PDA rather
    /// than left unset so that a future governance decision *could* enable
    /// freezing; no code path in this program exercises it today.
    #[account(
        init,
        payer = payer,
        mint::decimals = args.decimals,
        mint::authority = mint_authority,
        mint::freeze_authority = mint_authority,
        mint::token_program = token_program,
    )]
    pub mint: InterfaceAccount<'info, Mint>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn initialize_token(ctx: Context<InitializeToken>, args: InitializeTokenArgs) -> Result<()> {
    args.validate()?;

    let now = Clock::get()?.unix_timestamp;

    let config = &mut ctx.accounts.config;
    config.admin = ctx.accounts.admin.key();
    config.pending_admin = None;
    config.mint = ctx.accounts.mint.key();
    config.mint_authority_bump = ctx.bumps.mint_authority;
    config.bump = ctx.bumps.config;
    config.paused = false;
    config.total_minted = 0;
    config.total_burned = 0;
    config.minter_count = 0;

    emit!(TokenInitialized {
        config: config.key(),
        mint: config.mint,
        admin: config.admin,
        decimals: args.decimals,
        timestamp: now,
    });

    Ok(())
}
