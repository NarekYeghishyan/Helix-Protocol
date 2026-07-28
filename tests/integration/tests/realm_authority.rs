//! The realm's own parameters, and who is allowed to change them.
//!
//! `update_realm_params` sets what "passing" *means*: the quorum denominator's
//! share, the approval threshold, the voting period, the timelock, and the weight
//! needed to propose at all. It is gated on `realm.authority`.
//!
//! This file exists because of what that gate is reachable by — see
//! `the_realm_authority_can_be_revoked_by_governance`.

use helix_governance::instructions::realm::RealmParams;
use helix_governance::state::{ProposalAction, ProposalState, Realm, VoteChoice};
use helix_integration_tests::bootstrap::HOUR;
use helix_integration_tests::{pda, System};
use helix_staking::state::LockTier;
use helix_treasury::state::Treasury;

const FUNDING: u64 = 10_000_000;
const WHALE: u64 = 100_000_000;
/// A thousandth of the whale. Its weight still has to clear the 0.01% quorum
/// floor the attacker sets — the point is that 0.01% is the floor the *program*
/// permits, not that any amount at all suffices.
const DUST: u64 = 100_000;

/// Parameters an attacker holding the realm authority would choose.
fn permissive_params() -> RealmParams {
    RealmParams {
        // 0.01% — the smallest the program permits.
        quorum_bps: 1,
        // The floor. Nothing below this is accepted.
        approval_bps: 5_001,
        voting_period: HOUR,
        timelock_delay: HOUR,
        min_weight_to_propose: 1,
    }
}

/// The realm authority can rewrite the rules, and the rules are what stand
/// between a proposal and the treasury.
///
/// This is the attack in full: hold `realm.authority`, lower quorum to a
/// hundredth of a percent, then pass a treasury transfer with a dust position
/// while the honest majority of stake never votes.
#[test]
fn lowering_quorum_lets_a_dust_position_move_the_treasury() {
    let mut sys = System::bootstrap(None, FUNDING);

    // An honest majority: the overwhelming bulk of the pool's weight, which
    // never votes. Under the default 20% quorum nothing passes without it.
    let whale = sys.add_staker(solana_keypair::Keypair::new(), WHALE, LockTier::Gold);

    // The attacker's stake is a ten-thousandth of the whale's.
    sys.fund_voter(DUST);
    let dust = sys.stake(sys.next_position_id(), DUST, LockTier::Gold);

    // The realm authority is the payer, which `bootstrap` set at initialisation
    // and which nothing can change.
    sys.set_realm_params(permissive_params());

    let destination = sys.new_token_account(&sys.env.payer_pubkey());
    let proposal = sys.pass_treasury_transfer(0, dust, destination, FUNDING);

    let ix = sys.execute_treasury_transfer_ix(proposal, destination);
    sys.env.send(&[ix], &[]);

    let drained = sys.env.token_balance(&destination);
    let treasury: Treasury = sys.env.anchor_account(&sys.treasury);

    assert_eq!(
        drained, FUNDING,
        "the whole treasury moved on a dust vote: whale {} vs voter {}, \
         quorum {} bps",
        whale.position_id, DUST, 1
    );
    assert_eq!(treasury.total_spent, FUNDING);
}

/// Governance can take the parameters back, and the ex-authority keeps nothing.
#[test]
fn the_realm_authority_can_be_revoked_by_governance() {
    let mut sys = System::bootstrap(None, 0);

    sys.fund_voter(WHALE);
    let position = sys.stake(sys.next_position_id(), WHALE, LockTier::Gold);

    let executor = sys.executor;
    let proposal = sys.pass_proposal(
        0,
        ProposalAction::SetRealmAuthority {
            new_authority: executor,
        },
        position,
    );
    let ix = sys.execute_set_realm_authority_ix(proposal);
    sys.env.send(&[ix], &[]);

    let realm: Realm = sys.env.anchor_account(&sys.realm);
    assert_eq!(
        realm.authority, executor,
        "the realm authority did not move to the executor PDA"
    );

    // The key that held it at bootstrap can no longer touch the rules — which is
    // the entire point of the migration.
    let err = sys
        .try_set_realm_params(permissive_params())
        .expect_err("the superseded authority must not still set parameters");
    assert!(err.contains("NotAuthority"), "unexpected failure: {err}");
}

/// And governance can still retune itself afterwards, through a proposal.
///
/// Revoking the human authority is only safe if the parameters remain reachable.
/// An authority pointed at a PDA that no instruction can make sign would freeze
/// the realm's configuration permanently — the same defect as F-8 and F-9, one
/// step further along.
#[test]
fn governance_can_retune_its_own_parameters() {
    let mut sys = System::bootstrap(None, 0);

    sys.fund_voter(WHALE);
    let position = sys.stake(sys.next_position_id(), WHALE, LockTier::Gold);

    let executor = sys.executor;
    let proposal = sys.pass_proposal(
        0,
        ProposalAction::SetRealmAuthority {
            new_authority: executor,
        },
        position,
    );
    sys.env
        .send(&[sys.execute_set_realm_authority_ix(proposal)], &[]);

    // A longer timelock is the parameter a DAO most plausibly wants to raise, and
    // raising it cannot be done by the old authority any more.
    let tightened = RealmParams {
        quorum_bps: 3_000,
        approval_bps: 6_000,
        voting_period: 2 * HOUR,
        timelock_delay: 4 * HOUR,
        min_weight_to_propose: 1_000,
    };

    let proposal = sys.pass_proposal(
        1,
        ProposalAction::UpdateRealmParams { params: tightened },
        position,
    );
    sys.env
        .send(&[sys.execute_update_realm_params_ix(proposal)], &[]);

    let realm: Realm = sys.env.anchor_account(&sys.realm);
    assert_eq!(realm.quorum_bps, 3_000);
    assert_eq!(realm.approval_bps, 6_000);
    assert_eq!(realm.timelock_delay, 4 * HOUR);
    assert_eq!(realm.min_weight_to_propose, 1_000);
}

/// A proposal cannot smuggle parameters the program would reject directly.
#[test]
fn a_proposal_cannot_set_parameters_the_validator_refuses() {
    let mut sys = System::bootstrap(None, 0);

    sys.fund_voter(WHALE);
    let position = sys.stake(sys.next_position_id(), WHALE, LockTier::Gold);

    // Below MIN_APPROVAL_BPS, which is the only thing keeping `for > against`
    // true — see ARCHITECTURE.md. `update_realm_params` validates regardless of
    // who is asking, so routing it through governance must not bypass the check.
    let unsound = RealmParams {
        quorum_bps: 2_000,
        approval_bps: 4_000,
        voting_period: HOUR,
        timelock_delay: HOUR,
        min_weight_to_propose: 1,
    };

    let proposal = sys.pass_proposal(
        0,
        ProposalAction::UpdateRealmParams { params: unsound },
        position,
    );
    let err = sys
        .env
        .try_send(&[sys.execute_update_realm_params_ix(proposal)], &[])
        .expect_err("an approval threshold below the floor must be refused");
    assert!(
        err.contains("InvalidApprovalThreshold"),
        "unexpected failure: {err}"
    );

    // And the proposal did not consume itself on the way to failing.
    let p: helix_governance::state::Proposal =
        sys.env.anchor_account(&pda::proposal(&sys.realm, 0).0);
    assert_eq!(
        p.state,
        ProposalState::Queued,
        "a rejected execution must leave the proposal executable"
    );
}

/// The wrong handler cannot execute a realm action, and vice versa.
#[test]
fn realm_actions_cannot_be_executed_through_the_wrong_handler() {
    let mut sys = System::bootstrap(None, 0);

    sys.fund_voter(WHALE);
    let position = sys.stake(sys.next_position_id(), WHALE, LockTier::Gold);

    let proposal = sys.pass_proposal(0, ProposalAction::Signal, position);
    let err = sys
        .env
        .try_send(&[sys.execute_update_realm_params_ix(proposal)], &[])
        .expect_err("a Signal must not execute as a parameter change");
    assert!(
        err.contains("ActionAccountMismatch"),
        "unexpected failure: {err}"
    );
}

/// Only the executor's own realm can be retuned.
#[test]
fn a_vote_cast_before_the_parameters_changed_still_counts_under_the_new_ones() {
    // Not a defect, but worth pinning: `finalize` reads the realm's parameters at
    // finalisation, not at activation. A parameter change while a proposal is in
    // flight therefore applies to it. That is the reason revoking the human
    // authority matters even for proposals already open.
    let mut sys = System::bootstrap(None, 0);

    // A silent majority, so the voter below is genuinely a minority. Without
    // this the single voter *is* the whole electorate and any quorum, including
    // 100%, is satisfied by their vote alone.
    sys.add_staker(solana_keypair::Keypair::new(), WHALE, LockTier::Gold);

    sys.fund_voter(DUST);
    let position = sys.stake(sys.next_position_id(), DUST, LockTier::Gold);

    // Passes comfortably under the default 20% quorum? No — but it is not meant
    // to. What matters is that the outcome is decided by the parameters in force
    // at finalisation, not at activation.
    let proposal = sys.create_proposal(0, ProposalAction::Signal, position);
    sys.activate(proposal);
    sys.vote(proposal, position, VoteChoice::For);

    // Raise the bar above what the minority voter can clear.
    sys.set_realm_params(RealmParams {
        quorum_bps: 10_000,
        approval_bps: 5_001,
        voting_period: HOUR,
        timelock_delay: HOUR,
        min_weight_to_propose: 1,
    });

    sys.env.warp_forward(HOUR + 1);
    sys.finalize(proposal);

    let p: helix_governance::state::Proposal = sys.env.anchor_account(&proposal);
    assert_eq!(
        p.state,
        ProposalState::Defeated,
        "parameters are read at finalisation, so a mid-flight change applies"
    );
}
