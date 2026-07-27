//! Vesting streams: created by governance, claimed by the beneficiary,
//! revocable by governance but never retroactively.

use anchor_lang::prelude::*;
use anchor_spl::token_interface::{self, Mint, TokenAccount, TokenInterface, TransferChecked};

use crate::constants::{
    MIN_STREAM_DURATION, STREAM_SEED, TREASURY_SEED, VAULT_AUTHORITY_SEED, VAULT_SEED,
};
use crate::errors::TreasuryError;
use crate::events::{StreamClaimed, StreamCreated, StreamRevoked};
use crate::state::{Treasury, VestingStream};

#[derive(Accounts)]
#[instruction(stream_id: u64)]
pub struct CreateStream<'info> {
    #[account(
        mut,
        seeds = [TREASURY_SEED, treasury.mint.as_ref()],
        bump = treasury.bump,
        has_one = governance_executor @ TreasuryError::NotGovernanceExecutor,
        has_one = vault,
    )]
    pub treasury: Box<Account<'info, Treasury>>,

    pub governance_executor: Signer<'info>,

    #[account(mut)]
    pub payer: Signer<'info>,

    /// CHECK: recorded as the only address permitted to claim this stream.
    pub beneficiary: UncheckedAccount<'info>,

    #[account(
        init,
        payer = payer,
        space = 8 + VestingStream::INIT_SPACE,
        seeds = [
            STREAM_SEED,
            treasury.key().as_ref(),
            stream_id.to_le_bytes().as_ref(),
        ],
        bump,
    )]
    pub stream: Box<Account<'info, VestingStream>>,

    #[account(
        seeds = [VAULT_SEED, treasury.key().as_ref()],
        bump,
    )]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,

    pub system_program: Program<'info, System>,
}

/// Commits `total_amount` to a linear schedule for `beneficiary`.
///
/// The tokens are not moved — they stay in the vault and are recorded in
/// `committed_to_streams`, which `spend` subtracts from the spendable balance.
/// Escrowing into a separate account per stream would work too, but it costs an
/// extra token account per beneficiary and makes the treasury's true balance
/// harder to read. The commitment counter achieves the same guarantee.
pub fn create_stream(
    ctx: Context<CreateStream>,
    stream_id: u64,
    total_amount: u64,
    start_ts: i64,
    cliff_ts: i64,
    end_ts: i64,
) -> Result<()> {
    require!(total_amount > 0, TreasuryError::ZeroAmount);
    require!(
        start_ts <= cliff_ts && cliff_ts <= end_ts,
        TreasuryError::InvalidVestingSchedule
    );
    require!(
        end_ts.saturating_sub(start_ts) >= MIN_STREAM_DURATION,
        TreasuryError::StreamTooShort
    );

    let now = Clock::get()?.unix_timestamp;

    require_eq!(
        stream_id,
        ctx.accounts.treasury.stream_count,
        TreasuryError::MathOverflow
    );

    // The new commitment must be backed by tokens actually present and not
    // already promised elsewhere. Without this a treasury could promise the same
    // balance to several beneficiaries and only the first to claim would be paid.
    let available = ctx.accounts.treasury.uncommitted(ctx.accounts.vault.amount);
    require!(
        total_amount <= available,
        TreasuryError::InsufficientUncommittedBalance
    );

    let stream = &mut ctx.accounts.stream;
    stream.treasury = ctx.accounts.treasury.key();
    stream.beneficiary = ctx.accounts.beneficiary.key();
    stream.stream_id = stream_id;
    stream.total_amount = total_amount;
    stream.claimed = 0;
    stream.start_ts = start_ts;
    stream.cliff_ts = cliff_ts;
    stream.end_ts = end_ts;
    stream.revoked = false;
    stream.revoked_at = 0;
    stream.bump = ctx.bumps.stream;

    let treasury = &mut ctx.accounts.treasury;
    treasury.committed_to_streams = treasury
        .committed_to_streams
        .checked_add(total_amount)
        .ok_or(TreasuryError::MathOverflow)?;
    treasury.stream_count = treasury
        .stream_count
        .checked_add(1)
        .ok_or(TreasuryError::MathOverflow)?;

    emit!(StreamCreated {
        treasury: treasury.key(),
        stream: stream.key(),
        beneficiary: stream.beneficiary,
        stream_id,
        total_amount,
        start_ts,
        cliff_ts,
        end_ts,
        timestamp: now,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct ClaimStream<'info> {
    #[account(
        mut,
        seeds = [TREASURY_SEED, treasury.mint.as_ref()],
        bump = treasury.bump,
        has_one = vault,
        has_one = mint,
    )]
    pub treasury: Box<Account<'info, Treasury>>,

    #[account(
        mut,
        seeds = [
            STREAM_SEED,
            treasury.key().as_ref(),
            stream.stream_id.to_le_bytes().as_ref(),
        ],
        bump = stream.bump,
        has_one = treasury,
        has_one = beneficiary @ TreasuryError::NotBeneficiary,
    )]
    pub stream: Box<Account<'info, VestingStream>>,

    pub beneficiary: Signer<'info>,

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
        token::authority = beneficiary,
        token::token_program = token_program,
    )]
    pub beneficiary_token_account: Box<InterfaceAccount<'info, TokenAccount>>,

    /// CHECK: signs for the vault; identity fixed by seeds.
    #[account(
        seeds = [VAULT_AUTHORITY_SEED, treasury.key().as_ref()],
        bump = treasury.vault_authority_bump,
    )]
    pub vault_authority: UncheckedAccount<'info>,

    pub token_program: Interface<'info, TokenInterface>,
}

/// Withdraws everything vested and not yet claimed.
///
/// Works on a revoked stream too, for whatever had vested at the moment of
/// revocation — a revoke stops future accrual, it does not confiscate.
pub fn claim_stream(ctx: Context<ClaimStream>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;

    let amount = ctx.accounts.stream.claimable_at(now)?;
    require!(amount > 0, TreasuryError::NothingClaimable);

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
                to: ctx.accounts.beneficiary_token_account.to_account_info(),
                authority: ctx.accounts.vault_authority.to_account_info(),
            },
            signer_seeds,
        ),
        amount,
        ctx.accounts.mint.decimals,
    )?;

    let stream = &mut ctx.accounts.stream;
    stream.claimed = stream
        .claimed
        .checked_add(amount)
        .ok_or(TreasuryError::MathOverflow)?;

    // The claimed portion is no longer a commitment — it has been paid.
    let treasury = &mut ctx.accounts.treasury;
    treasury.committed_to_streams = treasury
        .committed_to_streams
        .checked_sub(amount)
        .ok_or(TreasuryError::MathOverflow)?;

    emit!(StreamClaimed {
        treasury: treasury_key,
        stream: ctx.accounts.stream.key(),
        beneficiary: ctx.accounts.beneficiary.key(),
        amount,
        total_claimed: ctx.accounts.stream.claimed,
        timestamp: now,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct RevokeStream<'info> {
    #[account(
        mut,
        seeds = [TREASURY_SEED, treasury.mint.as_ref()],
        bump = treasury.bump,
        has_one = governance_executor @ TreasuryError::NotGovernanceExecutor,
    )]
    pub treasury: Box<Account<'info, Treasury>>,

    pub governance_executor: Signer<'info>,

    #[account(
        mut,
        seeds = [
            STREAM_SEED,
            treasury.key().as_ref(),
            stream.stream_id.to_le_bytes().as_ref(),
        ],
        bump = stream.bump,
        has_one = treasury,
    )]
    pub stream: Box<Account<'info, VestingStream>>,
}

/// Stops future accrual on a stream.
///
/// Already-vested tokens stay claimable; only the unvested remainder returns to
/// the treasury's spendable balance. Revoking twice is refused so the freeze
/// timestamp cannot be moved later to reduce what the beneficiary has earned.
pub fn revoke_stream(ctx: Context<RevokeStream>) -> Result<()> {
    require!(!ctx.accounts.stream.revoked, TreasuryError::StreamRevoked);

    let now = Clock::get()?.unix_timestamp;

    let stream = &mut ctx.accounts.stream;
    stream.revoked = true;
    stream.revoked_at = now;

    // Evaluated after setting `revoked_at`, so this is the amount frozen at this
    // instant rather than at the end of the original schedule.
    let unvested = stream.unvested_remainder()?;
    let vested_retained = stream.claimable_at(now)?;

    let treasury = &mut ctx.accounts.treasury;
    treasury.committed_to_streams = treasury
        .committed_to_streams
        .checked_sub(unvested)
        .ok_or(TreasuryError::MathOverflow)?;

    emit!(StreamRevoked {
        treasury: treasury.key(),
        stream: ctx.accounts.stream.key(),
        beneficiary: ctx.accounts.stream.beneficiary,
        vested_retained,
        unvested_returned: unvested,
        timestamp: now,
    });

    Ok(())
}
