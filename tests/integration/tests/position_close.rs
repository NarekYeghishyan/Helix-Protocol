//! F-7 — reclaiming the rent of a fully exited position.
//!
//! The instruction itself is four lines of guard and an Anchor `close`. What
//! deserves runtime tests is the blast radius: closing an account is the one
//! operation that can make a *later* transaction mean something different,
//! because it frees an address that seeds a PDA which governance uses as the
//! electorate boundary.
//!
//! So the tests here are less about "does the rent come back" than about the
//! things that must stay true afterwards.

use anchor_lang::prelude::Pubkey;
use helix_governance::state::{Proposal, ProposalAction, VoteChoice};
use helix_integration_tests::bootstrap::System;
use helix_staking::state::{LockTier, Pool, Position};
use solana_signer::Signer as _;

const STAKE: u64 = 1_000_000;

/// Opens a position, waits out the lock, and withdraws all of it.
fn exited(sys: &mut System, tier: LockTier) -> (Pubkey, u64) {
    let id = sys.next_position_id();
    let position = sys.stake(id, STAKE, tier);
    sys.env.warp_forward(tier.duration() + 1);
    let ix = sys.unstake_ix(position, STAKE);
    let voter = sys.voter.insecure_clone();
    sys.env.send(&[ix], &[&voter]);
    (position, id)
}

#[test]
fn closing_an_exited_position_returns_its_rent() {
    let mut sys = System::bootstrap(None, 0);
    let (position, _) = exited(&mut sys, LockTier::Bronze);

    let rent = sys
        .env
        .svm
        .get_account(&position)
        .expect("position exists")
        .lamports;
    assert!(rent > 0);
    let before = sys.env.svm.get_balance(&sys.voter.pubkey()).unwrap();

    let ix = sys.close_position_ix(position);
    let voter = sys.voter.insecure_clone();
    sys.env.send(&[ix], &[&voter]);

    // The account is gone, not merely zeroed. Anchor's `close` also writes the
    // closed-account discriminator, so a same-transaction revival attempt would
    // fail to deserialise as a `Position` — but the load-bearing check here is
    // that the lamports reached the owner who paid them at `stake`.
    assert!(
        sys.env
            .svm
            .get_account(&position)
            .is_none_or(|a| a.lamports == 0),
        "position account still funded after close"
    );
    let after = sys.env.svm.get_balance(&sys.voter.pubkey()).unwrap();
    assert_eq!(after - before, rent, "rent did not reach the owner in full");
}

#[test]
fn a_position_holding_principal_cannot_be_closed() {
    let mut sys = System::bootstrap(None, 0);
    let id = sys.next_position_id();
    let position = sys.stake(id, STAKE, LockTier::Bronze);

    let ix = sys.close_position_ix(position);
    let voter = sys.voter.insecure_clone();
    let err = sys.env.try_send(&[ix], &[&voter]).unwrap_err();
    assert!(err.contains("PositionNotEmpty"), "{err}");
}

#[test]
fn a_position_with_unclaimed_rewards_cannot_be_closed() {
    // The case that makes the guard worth having. Principal is out, so the
    // position looks finished and a UI would happily offer "close" — but the
    // reward credit lives on the account being deallocated, and closing would
    // destroy a claim the vault is still holding tokens against.
    let mut sys = System::bootstrap(None, 0);
    sys.fund_rewards(1_000_000);
    // 100/s for 5_000s commits 500_000 against a vault holding 1_000_000. The
    // solvency guard refuses anything it cannot cover for the full period.
    let end = sys.env.now() + 5_000;
    sys.set_reward_rate(100, end);

    let (position, _) = exited(&mut sys, LockTier::Bronze);

    let settled: Position = sys.env.anchor_account(&position);
    assert!(
        settled.pending_rewards > 0,
        "fixture did not accrue anything to protect"
    );
    assert_eq!(settled.amount, 0);
    assert_eq!(settled.weighted_amount, 0);

    let voter = sys.voter.insecure_clone();
    let err = sys
        .env
        .try_send(&[sys.close_position_ix(position)], &[&voter])
        .unwrap_err();
    assert!(err.contains("UnclaimedRewards"), "{err}");

    // Claiming clears the obstacle rather than working around it.
    sys.env.send(&[sys.claim_ix(position)], &[&voter]);
    sys.env.send(&[sys.close_position_ix(position)], &[&voter]);
}

#[test]
fn closing_does_not_free_the_position_id_for_reuse() {
    // The regression that matters, and the reason `close_position` leaves
    // `pool.position_count` alone.
    //
    // The counter is both the PDA seed and the snapshot governance takes at
    // activation to decide who was in the electorate (F-10). Decrementing it on
    // close would let the next `stake` land at the closed position's address
    // *and* beneath an existing snapshot — a position created after a proposal
    // opened, voting on it. That is precisely the hole F-10 closed.
    let mut sys = System::bootstrap(None, 0);
    let (position, id) = exited(&mut sys, LockTier::Bronze);

    let before: Pool = sys.env.anchor_account(&sys.pool);
    let voter = sys.voter.insecure_clone();
    sys.env.send(&[sys.close_position_ix(position)], &[&voter]);

    let after: Pool = sys.env.anchor_account(&sys.pool);
    assert_eq!(
        after.position_count, before.position_count,
        "close decremented the electorate boundary"
    );

    // Re-staking at the freed id is refused, so the address cannot be reoccupied.
    let err = sys
        .env
        .try_send(&[sys.stake_ix(id, STAKE, LockTier::Bronze)], &[&voter])
        .unwrap_err();
    assert!(err.contains("UnexpectedPositionId"), "{err}");

    // The next id is the one after the closed position, not the closed one.
    let next = sys.next_position_id();
    assert_eq!(next, id + 1);
    let reopened = sys.stake(next, STAKE, LockTier::Bronze);
    assert_ne!(reopened, position);
}

#[test]
fn a_stranger_cannot_close_someone_elses_position() {
    let mut sys = System::bootstrap(None, 0);
    let (position, _) = exited(&mut sys, LockTier::Bronze);

    let thief = solana_keypair::Keypair::new();
    sys.env
        .svm
        .airdrop(&thief.pubkey(), 1_000_000_000)
        .expect("airdrop");

    // Addressed as the thief's own: `has_one = owner` and the seeds both point
    // at the real owner, so this fails on the address before it fails on the
    // signature.
    let ix = sys.close_position_ix_for(&thief.pubkey(), position);
    let err = sys.env.try_send(&[ix], &[&thief]).unwrap_err();
    assert!(
        err.contains("ConstraintSeeds") || err.contains("ConstraintHasOne"),
        "{err}"
    );

    // And the position is still there for its owner to close.
    let voter = sys.voter.insecure_clone();
    sys.env.send(&[sys.close_position_ix(position)], &[&voter]);
}

#[test]
fn a_voter_cannot_close_out_from_under_a_live_proposal() {
    // The interaction a reviewer should worry about: a position is a vote's
    // weight, and closing it destroys the account governance counted. If that
    // were reachable while a proposal was open, the tally would reference
    // weight that no longer exists.
    //
    // It is not reachable, and the reason is worth pinning rather than
    // re-deriving. Two existing rules compose: voting needs
    // `lock_end >= voting_ends_at`, and exiting needs `now >= lock_end`. So the
    // earliest a voter can empty its position is already after the vote closed.
    // Neither rule was written for this, which is exactly why it deserves a test
    // — nothing enforces that the composition keeps holding.
    let mut sys = System::bootstrap(None, 0);
    let position = sys.stake(0, STAKE, LockTier::Gold);

    let proposal = sys.create_proposal(0, ProposalAction::Signal, position);
    sys.activate(proposal);
    sys.vote(proposal, position, VoteChoice::For);

    let tally: Proposal = sys.env.anchor_account(&proposal);
    assert!(tally.for_votes > 0);

    // Mid-vote: the principal is not withdrawable, so the position cannot even
    // become closable.
    let voter = sys.voter.insecure_clone();
    let err = sys
        .env
        .try_send(&[sys.unstake_ix(position, STAKE)], &[&voter])
        .unwrap_err();
    assert!(err.contains("PositionLocked"), "{err}");

    let err = sys
        .env
        .try_send(&[sys.close_position_ix(position)], &[&voter])
        .unwrap_err();
    assert!(err.contains("PositionNotEmpty"), "{err}");

    // Once the lock expires the vote is long finished, and closing is fine.
    sys.env.warp_forward(LockTier::Gold.duration() + 1);
    sys.env.send(&[sys.unstake_ix(position, STAKE)], &[&voter]);
    sys.env.send(&[sys.close_position_ix(position)], &[&voter]);

    // The tally is arithmetic recorded on the proposal, not a live read of the
    // positions that produced it, so destroying the voter changes nothing.
    let after: Proposal = sys.env.anchor_account(&proposal);
    assert_eq!(after.for_votes, tally.for_votes);
    assert_eq!(after.total_weight_snapshot, tally.total_weight_snapshot);
}
