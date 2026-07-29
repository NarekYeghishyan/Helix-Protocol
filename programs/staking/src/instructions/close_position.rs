//! Reclaiming the rent of a fully exited position.
//!
//! This is a separate instruction rather than a `close = owner` constraint on
//! [`crate::instructions::unstake::Unstake`], for two reasons. Anchor's `close`
//! is unconditional, so it cannot express "only when this withdrawal happens to
//! be the final one" — and putting it on the withdrawal path would make every
//! partial exit pay for a decision only the last one can make.
//!
//! # What must not happen here
//!
//! `pool.position_count` is **not** decremented. It is tempting — the count
//! reads like "positions currently open" — and it would reintroduce a fixed
//! High-severity bug. The counter is the seed of every position PDA *and* the
//! electorate boundary that
//! [`F-10`](../../../../docs/SECURITY-ASSESSMENT.md) installed: governance
//! records `pool.position_count` at activation and accepts a vote only from a
//! position whose id is below that snapshot. Recycling an id would let a
//! position created after activation land beneath an existing snapshot, and
//! vote on a proposal that was open before it existed. It would also let the
//! new position be initialised at the closed one's address.
//!
//! The counter means "positions ever opened", and only that reading is safe.

use anchor_lang::prelude::*;

use crate::constants::{POOL_SEED, POSITION_SEED};
use crate::errors::StakingError;
use crate::events::PositionClosed;
use crate::state::{Pool, Position};

#[derive(Accounts)]
pub struct ClosePosition<'info> {
    /// Read-only, and deliberately so: a closable position holds no principal,
    /// no weight and no unpaid rewards, so there is nothing about it left in
    /// the pool's books to adjust. If closing ever needed to write here, the
    /// guard below would be wrong.
    #[account(
        seeds = [POOL_SEED, pool.stake_mint.as_ref(), pool.reward_mint.as_ref()],
        bump = pool.bump,
    )]
    pub pool: Box<Account<'info, Pool>>,

    /// Receives the reclaimed rent, having paid it in `stake`.
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        mut,
        close = owner,
        seeds = [
            POSITION_SEED,
            pool.key().as_ref(),
            owner.key().as_ref(),
            position.position_id.to_le_bytes().as_ref(),
        ],
        bump = position.bump,
        has_one = pool @ StakingError::PositionPoolMismatch,
        has_one = owner,
    )]
    pub position: Box<Account<'info, Position>>,
}

/// Closes an empty position and returns its rent to the owner.
///
/// Refuses unless principal, weight and unclaimed rewards are all zero. There
/// is no settlement step first, and that is sound rather than an omission: a
/// position with zero `weighted_amount` earns zero for any future value of the
/// accumulator, so `pending_rewards` is already final. `settling_is_a_no_op_on_an_empty_position`
/// holds that argument to the actual arithmetic.
pub fn close_position(ctx: Context<ClosePosition>) -> Result<()> {
    let position = &ctx.accounts.position;
    position.ensure_closable()?;

    emit!(PositionClosed {
        pool: ctx.accounts.pool.key(),
        position: position.key(),
        owner: ctx.accounts.owner.key(),
        position_id: position.position_id,
        timestamp: Clock::get()?.unix_timestamp,
    });

    Ok(())
}
