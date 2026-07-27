//! Issuance and redemption.
//!
//! `mint_to` is the only path to new supply, and it requires three independent
//! things to line up: a registered and enabled [`Minter`], that minter's own
//! signature, and headroom under its per-epoch cap. The mint authority itself is
//! a PDA no key can reproduce.

use anchor_lang::prelude::*;
use anchor_spl::token_interface::{self, Burn, Mint, MintTo, TokenAccount, TokenInterface};

use crate::constants::{CONFIG_SEED, MINTER_SEED, MINT_AUTHORITY_SEED};
use crate::errors::TokenManagerError;
use crate::events::{TokensBurned, TokensMinted};
use crate::state::{Minter, TokenConfig};

#[derive(Accounts)]
pub struct MintTokens<'info> {
    #[account(
        mut,
        seeds = [CONFIG_SEED, mint.key().as_ref()],
        bump = config.bump,
        has_one = mint,
    )]
    pub config: Account<'info, TokenConfig>,

    #[account(
        mut,
        seeds = [MINTER_SEED, config.key().as_ref(), authority.key().as_ref()],
        bump = minter.bump,
        has_one = config,
        has_one = authority @ TokenManagerError::MinterDisabled,
    )]
    pub minter: Account<'info, Minter>,

    /// The registered minter, signing for itself. In the deployed system this is
    /// the staking program's reward PDA, signing via CPI.
    pub authority: Signer<'info>,

    #[account(mut)]
    pub mint: InterfaceAccount<'info, Mint>,

    /// CHECK: not deserialised — it only ever signs. Its authority over the mint
    /// is established by the `seeds`/`bump` constraint, which is what makes this
    /// safe without a type.
    #[account(
        seeds = [MINT_AUTHORITY_SEED, config.key().as_ref()],
        bump = config.mint_authority_bump,
    )]
    pub mint_authority: UncheckedAccount<'info>,

    #[account(mut, token::mint = mint, token::token_program = token_program)]
    pub recipient: InterfaceAccount<'info, TokenAccount>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub fn mint_to(ctx: Context<MintTokens>, amount: u64) -> Result<()> {
    require!(amount > 0, TokenManagerError::ZeroAmount);
    require!(!ctx.accounts.config.paused, TokenManagerError::Paused);

    let now = Clock::get()?.unix_timestamp;

    // Charge the epoch budget *before* the CPI. If the cap is breached this
    // returns early and no tokens move; doing it afterwards would leave a window
    // where the mint succeeded but the accounting rejected it.
    ctx.accounts.minter.accrue(amount, now)?;

    let config_key = ctx.accounts.config.key();
    let signer_seeds: &[&[&[u8]]] = &[&[
        MINT_AUTHORITY_SEED,
        config_key.as_ref(),
        &[ctx.accounts.config.mint_authority_bump],
    ]];

    token_interface::mint_to(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            MintTo {
                mint: ctx.accounts.mint.to_account_info(),
                to: ctx.accounts.recipient.to_account_info(),
                authority: ctx.accounts.mint_authority.to_account_info(),
            },
            signer_seeds,
        ),
        amount,
    )?;

    let config = &mut ctx.accounts.config;
    config.total_minted = config
        .total_minted
        .checked_add(amount)
        .ok_or(TokenManagerError::MathOverflow)?;

    emit!(TokensMinted {
        config: config_key,
        minter: ctx.accounts.minter.key(),
        recipient: ctx.accounts.recipient.key(),
        amount,
        total_minted: config.total_minted,
        timestamp: now,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct BurnTokens<'info> {
    #[account(
        mut,
        seeds = [CONFIG_SEED, mint.key().as_ref()],
        bump = config.bump,
        has_one = mint,
    )]
    pub config: Account<'info, TokenConfig>,

    #[account(mut)]
    pub mint: InterfaceAccount<'info, Mint>,

    #[account(
        mut,
        token::mint = mint,
        token::authority = owner,
        token::token_program = token_program,
    )]
    pub source: InterfaceAccount<'info, TokenAccount>,

    pub owner: Signer<'info>,

    pub token_program: Interface<'info, TokenInterface>,
}

/// Burns from the caller's own account.
///
/// Note the absence of a pause check. Burning reduces supply and can only ever
/// harm the caller, so blocking it during a pause would gain nothing and would
/// turn the pause switch into a freeze.
pub fn burn(ctx: Context<BurnTokens>, amount: u64) -> Result<()> {
    require!(amount > 0, TokenManagerError::ZeroAmount);

    token_interface::burn(
        CpiContext::new(
            ctx.accounts.token_program.key(),
            Burn {
                mint: ctx.accounts.mint.to_account_info(),
                from: ctx.accounts.source.to_account_info(),
                authority: ctx.accounts.owner.to_account_info(),
            },
        ),
        amount,
    )?;

    let config = &mut ctx.accounts.config;
    config.total_burned = config
        .total_burned
        .checked_add(amount)
        .ok_or(TokenManagerError::MathOverflow)?;

    emit!(TokensBurned {
        config: config.key(),
        source: ctx.accounts.source.key(),
        amount,
        total_burned: config.total_burned,
        timestamp: Clock::get()?.unix_timestamp,
    });

    Ok(())
}
