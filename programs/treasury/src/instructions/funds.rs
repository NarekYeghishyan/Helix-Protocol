//! Deposits and proposal-gated spends.

use anchor_lang::prelude::*;
use anchor_spl::token_interface::{self, Mint, TokenAccount, TokenInterface, TransferChecked};

use crate::constants::{TREASURY_SEED, VAULT_AUTHORITY_SEED, VAULT_SEED};
use crate::errors::TreasuryError;
use crate::events::{Deposited, Spent};
use crate::state::Treasury;

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(
        mut,
        seeds = [TREASURY_SEED, treasury.mint.as_ref()],
        bump = treasury.bump,
        has_one = vault,
        has_one = mint,
    )]
    pub treasury: Box<Account<'info, Treasury>>,

    /// Anyone may fund the treasury.
    #[account(mut)]
    pub depositor: Signer<'info>,

    pub mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        mut,
        token::mint = mint,
        token::authority = depositor,
        token::token_program = token_program,
    )]
    pub depositor_token_account: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [VAULT_SEED, treasury.key().as_ref()],
        bump,
        token::mint = mint,
        token::token_program = token_program,
    )]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,

    pub token_program: Interface<'info, TokenInterface>,
}

/// Moves tokens into the vault.
///
/// As everywhere else in this workspace, the credited figure is the observed
/// vault delta rather than the `amount` argument, so a Token-2022 transfer fee
/// cannot make the treasury's books disagree with its balance.
pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
    require!(amount > 0, TreasuryError::ZeroAmount);

    let balance_before = ctx.accounts.vault.amount;

    token_interface::transfer_checked(
        CpiContext::new(
            ctx.accounts.token_program.key(),
            TransferChecked {
                from: ctx.accounts.depositor_token_account.to_account_info(),
                mint: ctx.accounts.mint.to_account_info(),
                to: ctx.accounts.vault.to_account_info(),
                authority: ctx.accounts.depositor.to_account_info(),
            },
        ),
        amount,
        ctx.accounts.mint.decimals,
    )?;

    ctx.accounts.vault.reload()?;
    let credited = ctx
        .accounts
        .vault
        .amount
        .checked_sub(balance_before)
        .ok_or(TreasuryError::VaultBalanceMismatch)?;
    require!(credited > 0, TreasuryError::ZeroAfterFees);

    let now = Clock::get()?.unix_timestamp;
    let treasury = &mut ctx.accounts.treasury;
    treasury.total_deposited = treasury
        .total_deposited
        .checked_add(credited)
        .ok_or(TreasuryError::MathOverflow)?;

    emit!(Deposited {
        treasury: treasury.key(),
        depositor: ctx.accounts.depositor.key(),
        amount_credited: credited,
        total_deposited: treasury.total_deposited,
        timestamp: now,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct Spend<'info> {
    #[account(
        mut,
        seeds = [TREASURY_SEED, treasury.mint.as_ref()],
        bump = treasury.bump,
        has_one = governance_executor @ TreasuryError::NotGovernanceExecutor,
        has_one = vault,
        has_one = mint,
    )]
    pub treasury: Box<Account<'info, Treasury>>,

    /// The governance execution PDA. Only the governance program can produce
    /// this signature, and only while executing a proposal that passed quorum
    /// and cleared its timelock.
    pub governance_executor: Signer<'info>,

    pub mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        mut,
        seeds = [VAULT_SEED, treasury.key().as_ref()],
        bump,
        token::mint = mint,
        token::token_program = token_program,
    )]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        token::mint = mint,
        token::token_program = token_program,
    )]
    pub destination: Box<InterfaceAccount<'info, TokenAccount>>,

    /// CHECK: signs for the vault; identity fixed by seeds.
    #[account(
        seeds = [VAULT_AUTHORITY_SEED, treasury.key().as_ref()],
        bump = treasury.vault_authority_bump,
    )]
    pub vault_authority: UncheckedAccount<'info>,

    pub token_program: Interface<'info, TokenInterface>,
}

/// Transfers `amount` out of the vault.
///
/// Two limits apply on top of the governance signature:
///
/// 1. The per-epoch spend cap, so even a passed malicious proposal cannot empty
///    the treasury in one transaction.
/// 2. The uncommitted balance, so a spend cannot pay out tokens already promised
///    to a vesting stream (`INVARIANTS.md` §1.6).
pub fn spend(ctx: Context<Spend>, amount: u64) -> Result<()> {
    require!(amount > 0, TreasuryError::ZeroAmount);

    let now = Clock::get()?.unix_timestamp;
    let vault_balance = ctx.accounts.vault.amount;

    // Committed stream obligations are not spendable, however the vote went.
    let available = ctx.accounts.treasury.uncommitted(vault_balance);
    require!(
        amount <= available,
        TreasuryError::InsufficientUncommittedBalance
    );

    // Charge the budget before the transfer, so a rejected spend moves nothing.
    ctx.accounts.treasury.charge_epoch_budget(amount, now)?;

    let treasury_key = ctx.accounts.treasury.key();
    let signer_seeds: &[&[&[u8]]] = &[&[
        VAULT_AUTHORITY_SEED,
        treasury_key.as_ref(),
        &[ctx.accounts.treasury.vault_authority_bump],
    ]];

    token_interface::transfer_checked(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            TransferChecked {
                from: ctx.accounts.vault.to_account_info(),
                mint: ctx.accounts.mint.to_account_info(),
                to: ctx.accounts.destination.to_account_info(),
                authority: ctx.accounts.vault_authority.to_account_info(),
            },
            signer_seeds,
        ),
        amount,
        ctx.accounts.mint.decimals,
    )?;

    let treasury = &mut ctx.accounts.treasury;
    treasury.total_spent = treasury
        .total_spent
        .checked_add(amount)
        .ok_or(TreasuryError::MathOverflow)?;

    emit!(Spent {
        treasury: treasury_key,
        destination: ctx.accounts.destination.key(),
        amount,
        remaining_epoch_budget: treasury.remaining_budget(now)?,
        total_spent: treasury.total_spent,
        timestamp: now,
    });

    Ok(())
}
