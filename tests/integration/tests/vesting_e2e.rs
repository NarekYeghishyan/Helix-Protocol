//! Phase 2.7 — vesting streams end to end, and the proof that F-8 is fixed.
//!
//! Before the fix these tests could not be written at all. `create_stream`
//! requires the governance executor's signature, and `ProposalAction` had no
//! variant that produced it — so there was no transaction that could create a
//! stream. Nine unit tests covered arithmetic no caller could reach.
//!
//! Every test here therefore doubles as a regression test for that gap: if the
//! `CreateVestingStream` / `RevokeVestingStream` variants are removed, none of
//! this compiles, let alone passes.

use anchor_lang::prelude::Pubkey;
use helix_governance::state::ProposalAction;
use helix_integration_tests::bootstrap::System;
use helix_staking::state::LockTier;
use helix_treasury::state::{Treasury, VestingStream};
use solana_keypair::Keypair;
use solana_signer::Signer as _;

const STAKE: u64 = 1_000_000;
const TREASURY_FUNDING: u64 = 5_000_000;
const GRANT: u64 = 1_200_000;

const DAY: i64 = 86_400;
const MONTH: i64 = 30 * DAY;
const YEAR: i64 = 365 * DAY;

struct Grant {
    sys: System,
    position: Pubkey,
    beneficiary: Keypair,
    beneficiary_tokens: Pubkey,
    start: i64,
}

/// Passes and executes a CreateVestingStream proposal.
fn granted(cliff: i64, duration: i64) -> Grant {
    let mut sys = System::bootstrap(None, TREASURY_FUNDING);
    let position = sys.stake(0, STAKE, LockTier::Gold);

    let beneficiary = Keypair::new();
    sys.env
        .svm
        .airdrop(
            &beneficiary.pubkey(),
            10 * solana_native_token::LAMPORTS_PER_SOL,
        )
        .unwrap();
    let beneficiary_tokens = sys.new_token_account(&beneficiary.pubkey());

    let start = sys.env.now();
    let proposal = sys.pass_proposal(
        0,
        ProposalAction::CreateVestingStream {
            beneficiary: beneficiary.pubkey(),
            total_amount: GRANT,
            start_ts: start,
            cliff_ts: start + cliff,
            end_ts: start + duration,
        },
        position,
    );

    let ix = sys.execute_create_stream_ix(proposal, 0, beneficiary.pubkey());
    sys.env.send(&[ix], &[]);

    Grant {
        sys,
        position,
        beneficiary,
        beneficiary_tokens,
        start,
    }
}

#[test]
fn governance_can_create_a_vesting_stream() {
    // F-8's regression test. This transaction was previously impossible to build.
    let g = granted(YEAR, 4 * YEAR);

    let (stream_addr, _) = helix_integration_tests::pda::stream(&g.sys.treasury, 0);
    let stream: VestingStream = g.sys.env.anchor_account(&stream_addr);

    assert_eq!(stream.beneficiary, g.beneficiary.pubkey());
    assert_eq!(stream.total_amount, GRANT);
    assert_eq!(stream.claimed, 0);
    assert!(!stream.revoked);

    // The grant is committed, so it is no longer spendable.
    let t: Treasury = g.sys.env.anchor_account(&g.sys.treasury);
    assert_eq!(t.committed_to_streams, GRANT);
    assert_eq!(t.stream_count, 1);
}

#[test]
fn nothing_is_claimable_before_the_cliff() {
    let mut g = granted(YEAR, 4 * YEAR);

    g.sys.env.warp_forward(MONTH * 6);

    let ix = g
        .sys
        .claim_stream_ix(0, g.beneficiary.pubkey(), g.beneficiary_tokens);
    let beneficiary = g.beneficiary.insecure_clone();
    let err = g
        .sys
        .env
        .try_send(&[ix], &[&beneficiary])
        .expect_err("claiming before the cliff must fail");
    assert!(
        err.contains("NothingClaimable"),
        "unexpected failure: {err}"
    );
    assert_eq!(g.sys.env.token_balance(&g.beneficiary_tokens), 0);
}

#[test]
fn the_cliff_releases_everything_accrued_since_start() {
    let mut g = granted(YEAR, 4 * YEAR);

    g.sys.env.warp_forward(YEAR);

    let ix = g
        .sys
        .claim_stream_ix(0, g.beneficiary.pubkey(), g.beneficiary_tokens);
    let beneficiary = g.beneficiary.insecure_clone();
    g.sys.env.send(&[ix], &[&beneficiary]);

    // Just past a year into four: about a quarter, released in one go.
    //
    // Computed from the schedule rather than hardcoded to GRANT/4. Driving the
    // proposal through its voting period and timelock consumes a couple of hours
    // of cluster time before the stream even starts, so "one year later" is a
    // little more than one year of vesting.
    let elapsed = g.sys.env.now() - g.start;
    let expected = (GRANT as u128 * elapsed as u128 / (4 * YEAR) as u128) as u64;

    let claimed = g.sys.env.token_balance(&g.beneficiary_tokens);
    assert_eq!(claimed, expected);
    assert!(
        claimed >= GRANT / 4,
        "the cliff must release the full accrual"
    );

    // The claimed portion stops being a commitment — it has been paid.
    let t: Treasury = g.sys.env.anchor_account(&g.sys.treasury);
    assert_eq!(t.committed_to_streams, GRANT - claimed);
}

#[test]
fn vesting_completes_and_never_overpays() {
    let mut g = granted(YEAR, 4 * YEAR);
    let beneficiary = g.beneficiary.insecure_clone();

    // Claim in stages, then long past the end.
    for _ in 0..3 {
        g.sys.env.warp_forward(YEAR);
        let ix = g
            .sys
            .claim_stream_ix(0, g.beneficiary.pubkey(), g.beneficiary_tokens);
        g.sys.env.send(&[ix], &[&beneficiary]);
    }

    g.sys.env.warp_forward(10 * YEAR);
    let ix = g
        .sys
        .claim_stream_ix(0, g.beneficiary.pubkey(), g.beneficiary_tokens);
    g.sys.env.send(&[ix], &[&beneficiary]);

    // Exactly the grant, never more, however long we wait.
    assert_eq!(g.sys.env.token_balance(&g.beneficiary_tokens), GRANT);

    let ix = g
        .sys
        .claim_stream_ix(0, g.beneficiary.pubkey(), g.beneficiary_tokens);
    assert!(
        g.sys.env.try_send(&[ix], &[&beneficiary]).is_err(),
        "a fully-claimed stream must have nothing left"
    );

    let t: Treasury = g.sys.env.anchor_account(&g.sys.treasury);
    assert_eq!(t.committed_to_streams, 0);
}

#[test]
fn only_the_beneficiary_can_claim() {
    let mut g = granted(0, 4 * YEAR);
    g.sys.env.warp_forward(YEAR);

    let thief = Keypair::new();
    g.sys
        .env
        .svm
        .airdrop(&thief.pubkey(), 10 * solana_native_token::LAMPORTS_PER_SOL)
        .unwrap();
    let thief_tokens = g.sys.new_token_account(&thief.pubkey());

    let ix = g.sys.claim_stream_ix(0, thief.pubkey(), thief_tokens);
    assert!(
        g.sys.env.try_send(&[ix], &[&thief]).is_err(),
        "a non-beneficiary must not be able to claim"
    );
    assert_eq!(g.sys.env.token_balance(&thief_tokens), 0);
}

#[test]
fn a_revoke_freezes_accrual_without_clawing_back() {
    // The property that makes revocation fair: forward-only.
    let mut g = granted(0, 4 * YEAR);
    let beneficiary = g.beneficiary.insecure_clone();

    g.sys.env.warp_forward(2 * YEAR);

    // A fresh position is required to vote. The original Gold lock was 180 days
    // and two years have passed, so it no longer satisfies
    // `lock_end >= voting_ends_at` — the flash-loan gate refusing an expired
    // lock, working exactly as intended.
    let voting_position = g.sys.stake(1, STAKE, LockTier::Gold);

    let proposal = g.sys.pass_proposal(
        1,
        ProposalAction::RevokeVestingStream { stream_id: 0 },
        voting_position,
    );
    let ix = g.sys.execute_revoke_stream_ix(proposal, 0);
    g.sys.env.send(&[ix], &[]);

    let (stream_addr, _) = helix_integration_tests::pda::stream(&g.sys.treasury, 0);
    let stream: VestingStream = g.sys.env.anchor_account(&stream_addr);
    assert!(stream.revoked);

    // Half the term elapsed, so half is still owed and remains claimable...
    g.sys.env.warp_forward(10 * YEAR);
    let ix = g
        .sys
        .claim_stream_ix(0, g.beneficiary.pubkey(), g.beneficiary_tokens);
    g.sys.env.send(&[ix], &[&beneficiary]);

    let claimed = g.sys.env.token_balance(&g.beneficiary_tokens);
    // Roughly half — the two hours of governance warping shift it slightly.
    assert!(
        (GRANT / 2..GRANT / 2 + GRANT / 1_000).contains(&claimed),
        "expected about half of {GRANT}, got {claimed}"
    );

    // ...and nothing accrued after the revoke, however long we waited.
    let ix = g
        .sys
        .claim_stream_ix(0, g.beneficiary.pubkey(), g.beneficiary_tokens);
    assert!(
        g.sys.env.try_send(&[ix], &[&beneficiary]).is_err(),
        "a revoked stream must not keep accruing"
    );
}

#[test]
fn the_unvested_remainder_returns_to_the_treasury() {
    let mut g = granted(0, 4 * YEAR);

    let before: Treasury = g.sys.env.anchor_account(&g.sys.treasury);
    assert_eq!(before.committed_to_streams, GRANT);

    g.sys.env.warp_forward(YEAR);

    // Fresh position: the original 180-day lock expired during the warp.
    let voting_position = g.sys.stake(1, STAKE, LockTier::Gold);

    let proposal = g.sys.pass_proposal(
        1,
        ProposalAction::RevokeVestingStream { stream_id: 0 },
        voting_position,
    );
    let ix = g.sys.execute_revoke_stream_ix(proposal, 0);
    g.sys.env.send(&[ix], &[]);

    // A quarter vested, so about three quarters returns to spendable balance.
    let after: Treasury = g.sys.env.anchor_account(&g.sys.treasury);
    assert!(
        after.committed_to_streams < before.committed_to_streams,
        "revoking must release the unvested remainder"
    );
    assert!(after.committed_to_streams <= GRANT / 4 + GRANT / 1_000);
}

#[test]
fn a_spend_cannot_touch_tokens_committed_to_a_stream() {
    // INVARIANTS.md §1.6 at runtime. Without the commitment counter a passed
    // proposal could drain the vault and leave the beneficiary's claim to fail.
    let mut g = granted(0, 4 * YEAR);

    let uncommitted = TREASURY_FUNDING - GRANT;
    let destination = g.sys.new_token_account(&g.sys.env.payer_pubkey());

    // One unit more than the free balance.
    let proposal = g.sys.pass_proposal(
        1,
        ProposalAction::TreasuryTransfer {
            destination,
            amount: uncommitted + 1,
        },
        g.position,
    );
    let ix = g.sys.execute_treasury_transfer_ix(proposal, destination);
    let err = g
        .sys
        .env
        .try_send(&[ix], &[])
        .expect_err("spending committed tokens must fail");
    assert!(
        err.contains("InsufficientUncommittedBalance"),
        "unexpected failure: {err}"
    );

    // Exactly the free balance is fine.
    let proposal = g.sys.pass_proposal(
        2,
        ProposalAction::TreasuryTransfer {
            destination,
            amount: uncommitted,
        },
        g.position,
    );
    let ix = g.sys.execute_treasury_transfer_ix(proposal, destination);
    g.sys.env.send(&[ix], &[]);
    assert_eq!(g.sys.env.token_balance(&destination), uncommitted);
}

#[test]
fn governance_can_change_the_treasury_spend_cap() {
    // Previously unreachable — see F-8.
    let mut sys = System::bootstrap(None, TREASURY_FUNDING);
    let position = sys.stake(0, STAKE, LockTier::Gold);

    let proposal = sys.pass_proposal(
        0,
        ProposalAction::SetTreasurySpendCap {
            new_cap: 12_345,
            epoch_duration: 2 * DAY,
        },
        position,
    );
    let ix = sys.execute_treasury_config_ix(proposal, true);
    sys.env.send(&[ix], &[]);

    let t: Treasury = sys.env.anchor_account(&sys.treasury);
    assert_eq!(t.epoch_spend_cap, 12_345);
    assert_eq!(t.epoch_duration, 2 * DAY);
}

#[test]
fn governance_can_hand_the_treasury_to_a_new_executor() {
    // The migration path, and the claim the treasury README makes. Previously
    // false: there was no way to invoke it.
    let mut sys = System::bootstrap(None, TREASURY_FUNDING);
    let position = sys.stake(0, STAKE, LockTier::Gold);
    let successor = Keypair::new().pubkey();

    let proposal = sys.pass_proposal(
        0,
        ProposalAction::SetGovernanceExecutor {
            new_executor: successor,
        },
        position,
    );
    let ix = sys.execute_treasury_config_ix(proposal, false);
    sys.env.send(&[ix], &[]);

    let t: Treasury = sys.env.anchor_account(&sys.treasury);
    assert_eq!(t.governance_executor, successor);

    // The old realm can no longer spend — it relinquished control.
    let destination = sys.new_token_account(&sys.env.payer_pubkey());
    let proposal = sys.pass_proposal(
        1,
        ProposalAction::TreasuryTransfer {
            destination,
            amount: 1_000,
        },
        position,
    );
    let ix = sys.execute_treasury_transfer_ix(proposal, destination);
    assert!(
        sys.env.try_send(&[ix], &[]).is_err(),
        "the superseded executor must no longer be able to spend"
    );
    assert_eq!(sys.env.token_balance(&destination), 0);
}

#[test]
fn a_stream_proposal_cannot_be_executed_through_the_wrong_handler() {
    let mut sys = System::bootstrap(None, TREASURY_FUNDING);
    let position = sys.stake(0, STAKE, LockTier::Gold);
    let destination = sys.new_token_account(&sys.env.payer_pubkey());

    let proposal = sys.pass_proposal(
        0,
        ProposalAction::SetTreasurySpendCap {
            new_cap: 1,
            epoch_duration: 2 * DAY,
        },
        position,
    );

    // Same proposal, wrong execute instruction.
    let ix = sys.execute_treasury_transfer_ix(proposal, destination);
    let err = sys
        .env
        .try_send(&[ix], &[])
        .expect_err("a spend-cap proposal must not move funds");
    assert!(
        err.contains("ActionAccountMismatch"),
        "unexpected failure: {err}"
    );
}

#[test]
fn a_substituted_beneficiary_is_refused() {
    let mut sys = System::bootstrap(None, TREASURY_FUNDING);
    let position = sys.stake(0, STAKE, LockTier::Gold);

    let approved = Keypair::new().pubkey();
    let attacker = Keypair::new().pubkey();

    let start = sys.env.now();
    let proposal = sys.pass_proposal(
        0,
        ProposalAction::CreateVestingStream {
            beneficiary: approved,
            total_amount: GRANT,
            start_ts: start,
            cliff_ts: start,
            end_ts: start + 4 * YEAR,
        },
        position,
    );

    let ix = sys.execute_create_stream_ix(proposal, 0, attacker);
    let err = sys
        .env
        .try_send(&[ix], &[])
        .expect_err("a substituted beneficiary must be refused");
    assert!(
        err.contains("ActionAccountMismatch"),
        "unexpected failure: {err}"
    );
}
