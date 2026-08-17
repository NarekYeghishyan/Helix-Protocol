//! Opening a position.
//!
//! The one subtlety here is that the amount transferred is not necessarily the
//! amount received. See [`stake`] for why that matters more than it looks.

use anchor_lang::prelude::*;
use anchor_spl::token_interface::{self, Mint, TokenAccount, TokenInterface, TransferChecked};

use crate::constants::{MIN_STAKE_AMOUNT, POOL_SEED, POSITION_SEED, STAKE_VAULT_SEED};
use crate::errors::StakingError;
use crate::events::Staked;
use crate::state::{LockTier, Pool, Position};

/// Note the `Box`ed accounts throughout.
///
/// Anchor generates a `try_accounts` that deserialises every account into a stack
/// frame, and SBF allows 4KB per frame. Token-2022 `Mint`/`TokenAccount` states
/// are large enough that five of them overflow it — the build reports
/// "Stack offset of 4104 exceeded max offset of 4096" and *still exits zero*,
/// leaving a program that may corrupt memory at runtime. Boxing moves the
/// deserialised accounts to the heap and keeps the frame small.
#[derive(Accounts)]
#[instruction(position_id: u64)]
pub struct Stake<'info> {
    #[account(
        mut,
        seeds = [POOL_SEED, pool.stake_mint.as_ref(), pool.reward_mint.as_ref()],
        bump = pool.bump,
        has_one = stake_vault,
        has_one = stake_mint,
    )]
    pub pool: Box<Account<'info, Pool>>,

    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        init,
        payer = owner,
        space = 8 + Position::INIT_SPACE,
        seeds = [
            POSITION_SEED,
            pool.key().as_ref(),
            owner.key().as_ref(),
            position_id.to_le_bytes().as_ref(),
        ],
        bump,
    )]
    pub position: Box<Account<'info, Position>>,

    pub stake_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        mut,
        token::mint = stake_mint,
        token::authority = owner,
        token::token_program = token_program,
    )]
    pub owner_token_account: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [STAKE_VAULT_SEED, pool.key().as_ref()],
        bump,
        token::mint = stake_mint,
        token::token_program = token_program,
    )]
    pub stake_vault: Box<InterfaceAccount<'info, TokenAccount>>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

/// Stakes `amount` under `tier`, opening a new position.
///
/// `position_id` must equal the pool's current `position_count`. Deriving the
/// PDA from a caller-supplied id while pinning that id to a monotonic counter
/// gives fixed-length seeds (no caller-controlled byte string to craft a
/// collision from) without needing to construct the seed from pool state.
pub fn stake(ctx: Context<Stake>, position_id: u64, amount: u64, tier: LockTier) -> Result<()> {
    require!(!ctx.accounts.pool.paused, StakingError::DepositsPaused);
    require!(amount >= MIN_STAKE_AMOUNT, StakingError::BelowMinimumStake);
    // Not `MathOverflow`, which this reported until `close_position` gave the
    // check a second way to be hit. Telling someone their arithmetic overflowed
    // sends them looking in the wrong place entirely.
    //
    // Two paths reach here, and neither is the one this most looks like. Losing
    // a race to another staker does *not*: the position PDA is seeded on
    // `position_id`, so the loser's account already exists and `init` refuses it
    // in `try_accounts` before this line runs, as system-program error 0. What
    // reaches here is an id *ahead* of the counter — a client computing it from
    // anything but a fresh read of `pool.position_count` — and an id *below* it
    // whose account `close_position` has since freed, which is the case that
    // must stay refused: recycling an id would let a new position land beneath
    // an existing governance snapshot (see `close_position.rs`).
    require_eq!(
        position_id,
        ctx.accounts.pool.position_count,
        StakingError::UnexpectedPositionId
    );

    let now = Clock::get()?.unix_timestamp;

    // Settle the pool at the old total_weighted before this stake changes it.
    // Doing this after would apply the new weight to rewards emitted before the
    // position existed.
    ctx.accounts.pool.update_rewards(now)?;

    // ---------------------------------------------------------------------
    // Credit the observed balance delta, never `amount`.
    //
    // With a Token-2022 transfer-fee extension active, `transfer_checked` moves
    // `amount` out of the sender but the vault receives `amount - fee`.
    // Crediting `amount` would break the solvency invariant (INVARIANTS.md §1.1)
    // immediately, and repeated deposit/withdraw cycles would drain the pool at
    // the other stakers' expense. This is invisible on a plain SPL mint, which
    // is exactly why it gets missed.
    // ---------------------------------------------------------------------
    let balance_before = ctx.accounts.stake_vault.amount;

    token_interface::transfer_checked(
        CpiContext::new(
            ctx.accounts.token_program.key(),
            TransferChecked {
                from: ctx.accounts.owner_token_account.to_account_info(),
                mint: ctx.accounts.stake_mint.to_account_info(),
                to: ctx.accounts.stake_vault.to_account_info(),
                authority: ctx.accounts.owner.to_account_info(),
            },
        ),
        amount,
        ctx.accounts.stake_mint.decimals,
    )?;

    ctx.accounts.stake_vault.reload()?;
    let credited = ctx
        .accounts
        .stake_vault
        .amount
        .checked_sub(balance_before)
        .ok_or(StakingError::VaultBalanceMismatch)?;

    require!(credited > 0, StakingError::ZeroAfterFees);

    let weighted = tier.apply_weight(credited)?;

    let position = &mut ctx.accounts.position;
    position.pool = ctx.accounts.pool.key();
    position.owner = ctx.accounts.owner.key();
    position.position_id = position_id;
    position.amount = credited;
    position.weighted_amount = weighted;
    position.tier = tier;
    position.lock_end = now
        .checked_add(tier.duration())
        .ok_or(StakingError::MathOverflow)?;
    // Start at the current watermark so the position earns nothing for time
    // before it existed.
    position.reward_per_token_paid = ctx.accounts.pool.reward_per_token;
    position.pending_rewards = 0;
    position.created_at = now;
    position.bump = ctx.bumps.position;

    let pool = &mut ctx.accounts.pool;
    pool.total_staked = pool
        .total_staked
        .checked_add(credited)
        .ok_or(StakingError::MathOverflow)?;
    pool.total_weighted = pool
        .total_weighted
        .checked_add(weighted)
        .ok_or(StakingError::MathOverflow)?;
    pool.position_count = pool
        .position_count
        .checked_add(1)
        .ok_or(StakingError::MathOverflow)?;

    emit!(Staked {
        pool: pool.key(),
        position: position.key(),
        owner: position.owner,
        position_id,
        amount_sent: amount,
        amount_credited: credited,
        weighted_amount: weighted,
        tier,
        lock_end: position.lock_end,
        timestamp: now,
    });

    Ok(())
}
