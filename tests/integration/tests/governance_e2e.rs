//! Phase 2.4 — the governance → treasury chain, executed rather than inspected.
//!
//! The central claim of the architecture is that **treasury funds move only under
//! the governance executor PDA's signature, and that signature exists only inside
//! the execution of a proposal that passed quorum and cleared its timelock.**
//!
//! Everything else in the design rests on that. Until now it was established by
//! reading `has_one` constraints. These tests execute it.

use helix_governance::state::{Proposal, ProposalAction, ProposalState, VoteChoice};
use helix_integration_tests::bootstrap::{System, HOUR};
use helix_staking::state::LockTier;
use helix_treasury::state::Treasury;
use solana_signer::Signer as _;

const STAKE: u64 = 1_000_000;
const TREASURY_FUNDING: u64 = 5_000_000;
const SPEND: u64 = 400_000;

// ===========================================================================
// The happy path
// ===========================================================================

#[test]
fn a_passed_proposal_moves_treasury_funds() {
    let mut sys = System::bootstrap(None, TREASURY_FUNDING);
    let position = sys.stake(0, STAKE, LockTier::Gold);

    let recipient = solana_keypair::Keypair::new();
    let destination = sys.new_token_account(&recipient.pubkey());

    let vault_before = sys.env.token_balance(&sys.treasury_vault);
    assert_eq!(sys.env.token_balance(&destination), 0);

    let proposal = sys.pass_treasury_transfer(0, position, destination, SPEND);

    // Everything up to here was governance; this is the moment funds move.
    let ix = sys.execute_treasury_transfer_ix(proposal, destination);
    sys.env.send(&[ix], &[]);

    assert_eq!(sys.env.token_balance(&destination), SPEND);
    assert_eq!(
        sys.env.token_balance(&sys.treasury_vault),
        vault_before - SPEND
    );

    let p: Proposal = sys.env.anchor_account(&proposal);
    assert_eq!(p.state, ProposalState::Executed);

    let t: Treasury = sys.env.anchor_account(&sys.treasury);
    assert_eq!(t.total_spent, SPEND);
}

#[test]
fn the_full_lifecycle_visits_every_state() {
    // INVARIANTS.md §4.6 — no state is skipped.
    let mut sys = System::bootstrap(None, TREASURY_FUNDING);
    let position = sys.stake(0, STAKE, LockTier::Gold);
    let destination = sys.new_token_account(&sys.env.payer_pubkey());

    let proposal = sys.create_proposal(
        0,
        ProposalAction::TreasuryTransfer {
            destination,
            amount: SPEND,
        },
        position,
    );

    let state = |s: &System| s.env.anchor_account::<Proposal>(&proposal).state;

    assert_eq!(state(&sys), ProposalState::Draft);

    sys.activate(proposal);
    assert_eq!(state(&sys), ProposalState::Voting);

    // Activation fixes the quorum denominator.
    let p: Proposal = sys.env.anchor_account(&proposal);
    assert_eq!(
        p.total_weight_snapshot,
        LockTier::Gold.apply_weight(STAKE).unwrap()
    );

    sys.vote(proposal, position, VoteChoice::For);
    let p: Proposal = sys.env.anchor_account(&proposal);
    assert_eq!(p.for_votes, p.total_weight_snapshot);

    sys.env.warp_forward(HOUR + 1);
    sys.finalize(proposal);
    assert_eq!(state(&sys), ProposalState::Succeeded);

    sys.queue(proposal);
    assert_eq!(state(&sys), ProposalState::Queued);
    let p: Proposal = sys.env.anchor_account(&proposal);
    assert!(
        p.eta > sys.env.now(),
        "eta must be in the future when queued"
    );

    sys.env.warp_forward(HOUR + 1);
    let ix = sys.execute_treasury_transfer_ix(proposal, destination);
    sys.env.send(&[ix], &[]);
    assert_eq!(state(&sys), ProposalState::Executed);
}

#[test]
fn a_defeated_proposal_moves_nothing() {
    let mut sys = System::bootstrap(None, TREASURY_FUNDING);
    let position = sys.stake(0, STAKE, LockTier::Gold);
    let destination = sys.new_token_account(&sys.env.payer_pubkey());

    let proposal = sys.create_proposal(
        0,
        ProposalAction::TreasuryTransfer {
            destination,
            amount: SPEND,
        },
        position,
    );
    sys.activate(proposal);
    sys.vote(proposal, position, VoteChoice::Against);

    sys.env.warp_forward(HOUR + 1);
    sys.finalize(proposal);

    let p: Proposal = sys.env.anchor_account(&proposal);
    assert_eq!(p.state, ProposalState::Defeated);

    // Queueing a defeated proposal is refused, so execution is unreachable.
    let queue = sys.advance_ix(proposal, false);
    assert!(sys.env.try_send(&[queue], &[]).is_err());
    assert_eq!(sys.env.token_balance(&destination), 0);
}

// ===========================================================================
// Negative tests — Phase 2.5. Each maps to a THREAT-MODEL entry.
// ===========================================================================

#[test]
fn treasury_rejects_a_spend_that_is_not_from_governance() {
    // INVARIANTS.md §5.1 / THREAT-MODEL A7. The single most important negative
    // test in the project: calling the treasury directly must fail.
    let mut sys = System::bootstrap(None, TREASURY_FUNDING);
    let attacker = solana_keypair::Keypair::new();
    sys.env
        .svm
        .airdrop(
            &attacker.pubkey(),
            10 * solana_native_token::LAMPORTS_PER_SOL,
        )
        .unwrap();
    let destination = sys.new_token_account(&attacker.pubkey());

    let ix = helix_integration_tests::TestEnv::ix(
        helix_treasury::ID,
        helix_treasury::accounts::Spend {
            treasury: sys.treasury,
            // The attacker signs in the executor's slot. The `has_one` on the
            // treasury is what rejects it.
            governance_executor: attacker.pubkey(),
            mint: sys.mint,
            vault: sys.treasury_vault,
            destination,
            vault_authority: sys.treasury_vault_authority,
            token_program: anchor_spl::token_2022::ID,
        },
        helix_treasury::instruction::Spend { amount: SPEND },
    );

    let err = sys
        .env
        .try_send(&[ix], &[&attacker])
        .expect_err("a direct treasury spend must fail");
    assert!(
        err.contains("NotGovernanceExecutor") || err.contains("2003"),
        "unexpected failure: {err}"
    );
    assert_eq!(sys.env.token_balance(&destination), 0);
    assert_eq!(sys.env.token_balance(&sys.treasury_vault), TREASURY_FUNDING);
}

#[test]
fn execution_before_the_timelock_elapses_is_refused() {
    // INVARIANTS.md §4.4 / THREAT-MODEL A8.
    let mut sys = System::bootstrap(None, TREASURY_FUNDING);
    let position = sys.stake(0, STAKE, LockTier::Gold);
    let destination = sys.new_token_account(&sys.env.payer_pubkey());

    let proposal = sys.create_proposal(
        0,
        ProposalAction::TreasuryTransfer {
            destination,
            amount: SPEND,
        },
        position,
    );
    sys.activate(proposal);
    sys.vote(proposal, position, VoteChoice::For);
    sys.env.warp_forward(HOUR + 1);
    sys.finalize(proposal);
    sys.queue(proposal);

    // Queued, but the timelock has not run. Deliberately do not warp.
    let ix = sys.execute_treasury_transfer_ix(proposal, destination);
    let err = sys
        .env
        .try_send(&[ix], &[])
        .expect_err("execution before eta must fail");
    assert!(
        err.contains("TimelockNotElapsed"),
        "unexpected failure: {err}"
    );
    assert_eq!(sys.env.token_balance(&destination), 0);
}

#[test]
fn a_proposal_cannot_execute_twice() {
    // INVARIANTS.md §4.5 / THREAT-MODEL A6 — replay would multiply the spend.
    let mut sys = System::bootstrap(None, TREASURY_FUNDING);
    let position = sys.stake(0, STAKE, LockTier::Gold);
    let destination = sys.new_token_account(&sys.env.payer_pubkey());

    let proposal = sys.pass_treasury_transfer(0, position, destination, SPEND);

    let ix = sys.execute_treasury_transfer_ix(proposal, destination);
    sys.env.send(std::slice::from_ref(&ix), &[]);
    assert_eq!(sys.env.token_balance(&destination), SPEND);

    // The state machine must refuse the second attempt.
    sys.env.warp_forward(1);
    let err = sys
        .env
        .try_send(&[ix], &[])
        .expect_err("second execution must fail");
    assert!(
        err.contains("InvalidProposalState"),
        "unexpected failure: {err}"
    );
    assert_eq!(
        sys.env.token_balance(&destination),
        SPEND,
        "funds moved twice"
    );
}

#[test]
fn a_position_cannot_vote_twice() {
    // INVARIANTS.md §4.1 — enforced by `init` on the vote record, so the second
    // attempt fails at account creation before any handler logic runs.
    let mut sys = System::bootstrap(None, 0);
    let position = sys.stake(0, STAKE, LockTier::Gold);

    let proposal = sys.create_proposal(0, ProposalAction::Signal, position);
    sys.activate(proposal);
    sys.vote(proposal, position, VoteChoice::For);

    let before: Proposal = sys.env.anchor_account(&proposal);

    sys.env.warp_forward(1);
    let ix = sys.vote_ix(proposal, position, VoteChoice::For);
    let voter = sys.voter.insecure_clone();
    assert!(
        sys.env.try_send(&[ix], &[&voter]).is_err(),
        "double voting must fail"
    );

    let after: Proposal = sys.env.anchor_account(&proposal);
    assert_eq!(
        after.for_votes, before.for_votes,
        "tally was double-counted"
    );
}

#[test]
fn a_flash_staked_position_cannot_vote() {
    // INVARIANTS.md §4.2 / THREAT-MODEL A1 — the flash-loan defence, executed.
    //
    // A Flexible position has lock_end == now, which is strictly less than any
    // live proposal's voting_ends_at, so it carries no weight however large it is.
    let mut sys = System::bootstrap(None, 0);

    // A committed position exists so the pool has a quorum denominator.
    let committed = sys.stake(0, STAKE, LockTier::Gold);
    let proposal = sys.create_proposal(0, ProposalAction::Signal, committed);
    sys.activate(proposal);

    // Now open a large, unlocked position — the borrowed-capital shape.
    let flash = sys.stake(1, STAKE * 5, LockTier::Flexible);

    let ix = sys.vote_ix(proposal, flash, VoteChoice::Against);
    let voter = sys.voter.insecure_clone();
    let err = sys
        .env
        .try_send(&[ix], &[&voter])
        .expect_err("an unlocked position must not be able to vote");
    assert!(
        err.contains("InsufficientLockDuration"),
        "unexpected failure: {err}"
    );

    let p: Proposal = sys.env.anchor_account(&proposal);
    assert_eq!(p.against_votes, 0, "flash-staked weight was counted");
}

#[test]
fn a_position_opened_after_the_snapshot_cannot_vote() {
    // INVARIANTS.md §4.3 / SECURITY-ASSESSMENT F-10. Found by the stateful
    // fuzzer, seed 10, and shrunk to two stakes and a vote.
    //
    // This is the case `a_flash_staked_position_cannot_vote` above does not
    // cover. §4.2 turns away *unlocked* capital; this position is locked for 180
    // days, so §4.2 waves it through. What it is not is part of the electorate
    // the quorum denominator was measured over — `total_weight_snapshot` was
    // taken at activation, before this position existed.
    //
    // The visible symptom is `for + against + abstain > total_weight_snapshot`.
    // The damage is quorum. Quorum is `votes × 10_000 >= snapshot × quorum_bps`,
    // so weight staked after the snapshot adds to the numerator and never
    // reaches the denominator: buy enough of it and a proposal clears a
    // threshold measured against an electorate that no longer exists.
    let mut sys = System::bootstrap(None, 0);

    let incumbent = sys.stake(0, STAKE, LockTier::Gold);
    let proposal = sys.create_proposal(0, ProposalAction::Signal, incumbent);
    sys.activate(proposal);

    let snapshot = sys
        .env
        .anchor_account::<Proposal>(&proposal)
        .total_weight_snapshot;

    // Five times the electorate, locked for 180 days — well past voting_ends_at,
    // so the flash-loan gate is satisfied.
    let latecomer = sys.stake(1, STAKE * 5, LockTier::Gold);

    let ix = sys.vote_ix(proposal, latecomer, VoteChoice::For);
    let voter = sys.voter.insecure_clone();
    let err = sys
        .env
        .try_send(&[ix], &[&voter])
        .expect_err("a position created after the snapshot must not vote");
    assert!(
        err.contains("PositionNotInSnapshot"),
        "unexpected failure: {err}"
    );

    let p: Proposal = sys.env.anchor_account(&proposal);
    assert_eq!(p.for_votes, 0, "post-snapshot weight was counted");
    assert!(
        p.total_votes().unwrap() <= snapshot,
        "§4.3: {} counted against a {snapshot} snapshot",
        p.total_votes().unwrap()
    );

    // And the incumbent — which *was* in the snapshot — still votes.
    sys.vote(proposal, incumbent, VoteChoice::For);
    let p: Proposal = sys.env.anchor_account(&proposal);
    assert_eq!(
        p.for_votes, snapshot,
        "the fix turned away the electorate along with the latecomer"
    );
}

#[test]
fn the_guardian_veto_prevents_execution() {
    // INVARIANTS.md §4.7 — the guardian's one power.
    let mut sys = System::bootstrap(None, TREASURY_FUNDING);
    let position = sys.stake(0, STAKE, LockTier::Gold);
    let destination = sys.new_token_account(&sys.env.payer_pubkey());

    let proposal = sys.pass_treasury_transfer(0, position, destination, SPEND);

    // Queued and past its timelock — one instruction away from executing.
    sys.cancel(proposal);
    let p: Proposal = sys.env.anchor_account(&proposal);
    assert_eq!(p.state, ProposalState::Cancelled);

    let ix = sys.execute_treasury_transfer_ix(proposal, destination);
    assert!(
        sys.env.try_send(&[ix], &[]).is_err(),
        "a cancelled proposal must not execute"
    );
    assert_eq!(sys.env.token_balance(&destination), 0);
}

#[test]
fn the_guardian_cannot_do_anything_but_cancel() {
    // INVARIANTS.md §4.7, second half — previously established only by
    // inspection. A guardian that could also pass proposals would be an admin key.
    let mut sys = System::bootstrap(None, TREASURY_FUNDING);
    let position = sys.stake(0, STAKE, LockTier::Gold);
    let destination = sys.new_token_account(&sys.env.payer_pubkey());
    let guardian = sys.guardian.insecure_clone();

    // Fund the guardian. Without this the vote below fails on rent for the vote
    // record rather than on authorisation — the test would pass while proving
    // nothing, which is how it read before the assertions were tightened.
    sys.env
        .svm
        .airdrop(
            &guardian.pubkey(),
            10 * solana_native_token::LAMPORTS_PER_SOL,
        )
        .unwrap();

    let proposal = sys.create_proposal(
        0,
        ProposalAction::TreasuryTransfer {
            destination,
            amount: SPEND,
        },
        position,
    );

    // The guardian cannot spend the treasury directly.
    let spend = helix_integration_tests::TestEnv::ix(
        helix_treasury::ID,
        helix_treasury::accounts::Spend {
            treasury: sys.treasury,
            governance_executor: guardian.pubkey(),
            mint: sys.mint,
            vault: sys.treasury_vault,
            destination,
            vault_authority: sys.treasury_vault_authority,
            token_program: anchor_spl::token_2022::ID,
        },
        helix_treasury::instruction::Spend { amount: SPEND },
    );
    assert!(
        sys.env.try_send(&[spend], &[&guardian]).is_err(),
        "guardian must not be able to spend"
    );

    // Nor vote with someone else's position. Note the guardian occupies the
    // `voter` slot here — signing the voter's instruction with the guardian's key
    // would fail at transaction signing and never reach the program, which would
    // test nothing.
    let (vote_record, _) = helix_integration_tests::pda::vote_record(&proposal, &position);
    let vote = helix_integration_tests::TestEnv::ix(
        helix_governance::ID,
        helix_governance::accounts::CastVote {
            realm: sys.realm,
            proposal,
            voter: guardian.pubkey(),
            position,
            vote_record,
            system_program: anchor_lang::system_program::ID,
        },
        helix_governance::instruction::CastVote {
            choice: VoteChoice::For,
        },
    );
    let err = sys
        .env
        .try_send(&[vote], &[&guardian])
        .expect_err("guardian must not be able to vote with another's position");
    assert!(
        err.contains("NotPositionOwner"),
        "unexpected failure: {err}"
    );

    assert_eq!(sys.env.token_balance(&sys.treasury_vault), TREASURY_FUNDING);
}

#[test]
fn executing_a_different_destination_than_the_proposal_named_is_refused() {
    // Execution reads its parameters from `proposal.action`, so a caller cannot
    // redirect an approved spend to themselves.
    let mut sys = System::bootstrap(None, TREASURY_FUNDING);
    let position = sys.stake(0, STAKE, LockTier::Gold);

    let approved = sys.new_token_account(&sys.env.payer_pubkey());
    let attacker = solana_keypair::Keypair::new();
    let attacker_account = sys.new_token_account(&attacker.pubkey());

    let proposal = sys.pass_treasury_transfer(0, position, approved, SPEND);

    // Same proposal, substituted destination.
    let ix = sys.execute_treasury_transfer_ix(proposal, attacker_account);
    let err = sys
        .env
        .try_send(&[ix], &[])
        .expect_err("a substituted destination must be refused");
    assert!(
        err.contains("ActionAccountMismatch"),
        "unexpected failure: {err}"
    );

    assert_eq!(sys.env.token_balance(&attacker_account), 0);
    assert_eq!(sys.env.token_balance(&approved), 0);
}

#[test]
fn a_signal_proposal_cannot_be_executed_as_a_treasury_transfer() {
    // The action enum is a closed set, and the execute instruction must verify
    // the variant it was handed rather than trusting the caller's choice.
    let mut sys = System::bootstrap(None, TREASURY_FUNDING);
    let position = sys.stake(0, STAKE, LockTier::Gold);
    let destination = sys.new_token_account(&sys.env.payer_pubkey());

    let proposal = sys.create_proposal(0, ProposalAction::Signal, position);
    sys.activate(proposal);
    sys.vote(proposal, position, VoteChoice::For);
    sys.env.warp_forward(HOUR + 1);
    sys.finalize(proposal);
    sys.queue(proposal);
    sys.env.warp_forward(HOUR + 1);

    let ix = sys.execute_treasury_transfer_ix(proposal, destination);
    let err = sys
        .env
        .try_send(&[ix], &[])
        .expect_err("a Signal proposal must not move funds");
    assert!(
        err.contains("ActionAccountMismatch"),
        "unexpected failure: {err}"
    );
    assert_eq!(sys.env.token_balance(&destination), 0);
}

#[test]
fn voting_after_the_window_closes_is_refused() {
    let mut sys = System::bootstrap(None, 0);
    let position = sys.stake(0, STAKE, LockTier::Gold);

    let proposal = sys.create_proposal(0, ProposalAction::Signal, position);
    sys.activate(proposal);

    sys.env.warp_forward(HOUR + 1);

    let ix = sys.vote_ix(proposal, position, VoteChoice::For);
    let voter = sys.voter.insecure_clone();
    let err = sys
        .env
        .try_send(&[ix], &[&voter])
        .expect_err("voting after close must fail");
    assert!(err.contains("VotingEnded"), "unexpected failure: {err}");
}

#[test]
fn finalizing_while_voting_is_still_open_is_refused() {
    let mut sys = System::bootstrap(None, 0);
    let position = sys.stake(0, STAKE, LockTier::Gold);

    let proposal = sys.create_proposal(0, ProposalAction::Signal, position);
    sys.activate(proposal);
    sys.vote(proposal, position, VoteChoice::For);

    let ix = sys.advance_ix(proposal, true);
    let err = sys
        .env
        .try_send(&[ix], &[])
        .expect_err("finalizing early must fail");
    assert!(err.contains("VotingStillOpen"), "unexpected failure: {err}");
}
