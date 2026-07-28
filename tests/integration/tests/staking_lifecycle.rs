//! Phase 2.2b — the staking withdrawal lifecycle at runtime.
//!
//! `claim` and `unstake` are the paths users depend on to get their money out.
//! The arithmetic behind them is unit-tested; what these cover is the token
//! movement and account bookkeeping around it, plus the two deliberate limits on
//! the pause switch that make it a pause rather than a freeze.

use helix_integration_tests::bootstrap::System;
use helix_staking::state::{Pool, Position};
use solana_signer::Signer as _;

const STAKE: u64 = 1_000_000;
const REWARD_FUNDING: u64 = 1_000_000;
const RATE: u64 = 100; // per second
const PERIOD: i64 = 5_000;

/// A pool with emissions running.
fn with_rewards() -> System {
    let mut sys = System::bootstrap(None, 0);
    sys.fund_rewards(REWARD_FUNDING);
    let end = sys.env.now() + PERIOD;
    sys.set_reward_rate(RATE, end);
    sys
}

#[test]
fn a_funded_pool_accepts_a_nonzero_reward_rate() {
    // Regression for F-2. The solvency guard previously used deposits as
    // liability, which made every non-zero rate unsettable — the pool could
    // never have paid rewards at all. This is that bug's runtime witness.
    let sys = with_rewards();
    let pool: Pool = sys.env.anchor_account(&sys.pool);
    assert_eq!(pool.reward_rate, RATE);
}

#[test]
fn an_unfundable_rate_is_refused() {
    let mut sys = System::bootstrap(None, 0);
    sys.fund_rewards(1_000);

    // 100/s for 5000s commits 500_000 against a vault holding 1_000.
    let end = sys.env.now() + PERIOD;
    let ix = sys.set_reward_rate_ix(RATE, end);
    let err = sys
        .env
        .try_send(&[ix], &[])
        .expect_err("an unfundable rate must be refused");
    assert!(
        err.contains("InsufficientRewardFunding"),
        "unexpected failure: {err}"
    );
}

#[test]
fn rewards_accrue_over_time_and_can_be_claimed() {
    let mut sys = with_rewards();
    let position = sys.stake(0, STAKE, helix_staking::state::LockTier::Flexible);

    let before = sys.env.token_balance(&sys.voter_tokens);
    sys.env.warp_forward(100);

    let ix = sys.claim_ix(position);
    sys.send_as_voter(ix);

    let gained = sys.env.token_balance(&sys.voter_tokens) - before;
    // Sole staker, so the whole emission for 100 seconds.
    assert_eq!(gained, 100 * RATE);

    let pos: Position = sys.env.anchor_account(&position);
    assert_eq!(pos.pending_rewards, 0, "claim must zero the credit");

    let pool: Pool = sys.env.anchor_account(&sys.pool);
    assert_eq!(pool.total_rewards_paid, gained);
}

#[test]
fn claiming_twice_in_the_same_slot_pays_once() {
    let mut sys = with_rewards();
    let position = sys.stake(0, STAKE, helix_staking::state::LockTier::Flexible);
    sys.env.warp_forward(100);

    let ix = sys.claim_ix(position);
    sys.send_as_voter(ix);
    let after_first = sys.env.token_balance(&sys.voter_tokens);

    // Nothing has accrued since, so there is nothing to claim.
    let ix = sys.claim_ix(position);
    let err = sys
        .try_as_voter(ix)
        .expect_err("a second claim with no accrual must fail");
    assert!(err.contains("NothingToClaim"), "unexpected failure: {err}");
    assert_eq!(sys.env.token_balance(&sys.voter_tokens), after_first);
}

#[test]
fn unstaking_before_the_lock_expires_is_refused() {
    let mut sys = System::bootstrap(None, 0);
    let position = sys.stake(0, STAKE, helix_staking::state::LockTier::Gold);

    let ix = sys.unstake_ix(position, STAKE);
    let err = sys
        .try_as_voter(ix)
        .expect_err("unstaking a locked position must fail");
    assert!(err.contains("PositionLocked"), "unexpected failure: {err}");

    // Principal is untouched.
    assert_eq!(sys.env.token_balance(&sys.stake_vault), STAKE);
}

#[test]
fn unstaking_after_the_lock_returns_principal() {
    let mut sys = System::bootstrap(None, 0);
    let position = sys.stake(0, STAKE, helix_staking::state::LockTier::Flexible);

    let before = sys.env.token_balance(&sys.voter_tokens);
    sys.env.warp_forward(1);

    let ix = sys.unstake_ix(position, STAKE);
    sys.send_as_voter(ix);

    assert_eq!(sys.env.token_balance(&sys.voter_tokens) - before, STAKE);
    assert_eq!(sys.env.token_balance(&sys.stake_vault), 0);

    let pool: Pool = sys.env.anchor_account(&sys.pool);
    assert_eq!(pool.total_staked, 0);
    assert_eq!(pool.total_weighted, 0);

    let pos: Position = sys.env.anchor_account(&position);
    assert_eq!(pos.amount, 0);
    assert_eq!(pos.weighted_amount, 0);
}

#[test]
fn partial_unstake_recomputes_weight_from_the_remainder() {
    // Weight is recomputed from the remaining principal rather than subtracted
    // proportionally, so `weighted == tier.apply_weight(amount)` stays exactly
    // true after any sequence of partial withdrawals.
    use helix_staking::state::LockTier;

    let mut sys = System::bootstrap(None, 0);
    let position = sys.stake(0, STAKE, LockTier::Flexible);
    sys.env.warp_forward(1);

    for _ in 0..3 {
        let ix = sys.unstake_ix(position, STAKE / 4);
        sys.send_as_voter(ix);

        let pos: Position = sys.env.anchor_account(&position);
        assert_eq!(
            pos.weighted_amount,
            LockTier::Flexible.apply_weight(pos.amount).unwrap()
        );

        let pool: Pool = sys.env.anchor_account(&sys.pool);
        assert_eq!(pool.total_staked, pos.amount);
        assert_eq!(pool.total_weighted, pos.weighted_amount);
        assert_eq!(sys.env.token_balance(&sys.stake_vault), pos.amount);
    }
}

#[test]
fn claiming_is_available_while_the_position_is_locked() {
    // INVARIANTS.md §6.1. Locking principal is the bargain the staker agreed to;
    // locking the yield on top of it is not.
    let mut sys = with_rewards();
    let position = sys.stake(0, STAKE, helix_staking::state::LockTier::Gold);

    sys.env.warp_forward(100);
    let before = sys.env.token_balance(&sys.voter_tokens);

    let ix = sys.claim_ix(position);
    sys.send_as_voter(ix);

    assert!(
        sys.env.token_balance(&sys.voter_tokens) > before,
        "a locked position must still be able to claim"
    );

    // The principal is still locked, though.
    let ix = sys.unstake_ix(position, STAKE);
    assert!(sys.try_as_voter(ix).is_err());
}

#[test]
fn pausing_blocks_deposits_but_never_traps_funds() {
    // INVARIANTS.md §6.4 — the property that makes this a pause and not a freeze.
    use helix_staking::state::LockTier;

    let mut sys = with_rewards();
    let position = sys.stake(0, STAKE, LockTier::Flexible);
    sys.env.warp_forward(100);

    sys.set_paused(true);

    // New exposure is refused...
    let ix = sys.stake_ix(1, STAKE, LockTier::Flexible);
    let err = sys
        .try_as_voter(ix)
        .expect_err("staking while paused must fail");
    assert!(err.contains("DepositsPaused"), "unexpected failure: {err}");

    // ...but the exits stay open.
    let before = sys.env.token_balance(&sys.voter_tokens);
    let ix = sys.claim_ix(position);
    sys.send_as_voter(ix);
    assert!(
        sys.env.token_balance(&sys.voter_tokens) > before,
        "pause must not block claiming"
    );

    let ix = sys.unstake_ix(position, STAKE);
    sys.send_as_voter(ix);
    assert_eq!(
        sys.env.token_balance(&sys.stake_vault),
        0,
        "pause must not block unstaking"
    );
}

#[test]
fn the_vault_stays_solvent_across_a_full_cycle() {
    // INVARIANTS.md §1.1 and §1.3 through stake → accrue → claim → unstake.
    use helix_staking::state::LockTier;

    let mut sys = with_rewards();

    let a = sys.stake(0, STAKE, LockTier::Flexible);
    let b = sys.stake(1, STAKE / 2, LockTier::Flexible);

    let check = |s: &System| {
        let pool: Pool = s.env.anchor_account(&s.pool);
        let pa: Position = s.env.anchor_account(&a);
        let pb: Position = s.env.anchor_account(&b);
        assert_eq!(
            pa.amount + pb.amount,
            s.env.token_balance(&s.stake_vault),
            "Σ position.amount != stake_vault.amount"
        );
        assert_eq!(pool.total_staked, pa.amount + pb.amount);
        assert_eq!(pool.total_weighted, pa.weighted_amount + pb.weighted_amount);
    };

    check(&sys);
    sys.env.warp_forward(200);
    check(&sys);

    let ix = sys.claim_ix(a);
    sys.send_as_voter(ix);
    check(&sys);

    let ix = sys.unstake_ix(b, STAKE / 4);
    sys.send_as_voter(ix);
    check(&sys);

    let ix = sys.unstake_ix(a, STAKE);
    sys.send_as_voter(ix);
    check(&sys);

    // Rewards paid must never exceed what was funded.
    let pool: Pool = sys.env.anchor_account(&sys.pool);
    assert!(
        pool.total_rewards_paid <= REWARD_FUNDING,
        "paid {} against {REWARD_FUNDING} funded",
        pool.total_rewards_paid
    );
    assert!(pool.total_rewards_paid <= pool.total_rewards_accrued);
}

#[test]
fn emissions_stop_at_the_period_end() {
    let mut sys = with_rewards();
    let position = sys.stake(0, STAKE, helix_staking::state::LockTier::Flexible);

    // Run well past the funded period.
    sys.env.warp_forward(PERIOD * 3);

    let before = sys.env.token_balance(&sys.voter_tokens);
    let ix = sys.claim_ix(position);
    sys.send_as_voter(ix);
    let gained = sys.env.token_balance(&sys.voter_tokens) - before;

    // Capped at the full period's emission, not the elapsed wall time.
    assert_eq!(gained, (PERIOD as u64) * RATE);
}

#[test]
fn a_staker_cannot_claim_another_stakers_position() {
    let mut sys = with_rewards();
    let position = sys.stake(0, STAKE, helix_staking::state::LockTier::Flexible);
    sys.env.warp_forward(100);

    let thief = solana_keypair::Keypair::new();
    sys.env
        .svm
        .airdrop(&thief.pubkey(), 10 * solana_native_token::LAMPORTS_PER_SOL)
        .unwrap();
    let thief_tokens = sys.new_token_account(&thief.pubkey());

    let (vault_authority, _) = helix_integration_tests::pda::vault_authority(&sys.pool);
    let ix = helix_integration_tests::TestEnv::ix(
        helix_staking::ID,
        helix_staking::accounts::Claim {
            pool: sys.pool,
            owner: thief.pubkey(),
            position,
            reward_mint: sys.mint,
            reward_vault: sys.reward_vault,
            owner_reward_account: thief_tokens,
            vault_authority,
            token_program: anchor_spl::token_2022::ID,
        },
        helix_staking::instruction::Claim {},
    );

    assert!(
        sys.env.try_send(&[ix], &[&thief]).is_err(),
        "claiming another staker's position must fail"
    );
    assert_eq!(sys.env.token_balance(&thief_tokens), 0);
}
