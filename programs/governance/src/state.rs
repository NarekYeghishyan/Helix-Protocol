//! Realm configuration, the proposal state machine, and vote records.

use anchor_lang::prelude::*;

use crate::constants::{BPS_DENOMINATOR, EXECUTION_GRACE_PERIOD, MAX_TITLE_LEN, MAX_URI_LEN};
use crate::errors::GovernanceError;

/// What a passed proposal actually does.
///
/// This is a closed enum rather than a blob of serialised instruction data.
/// General-purpose governance (SPL Governance, OpenZeppelin Governor) lets a
/// proposal carry arbitrary CPIs, which is more flexible and much harder to
/// reason about: a voter has to decode raw instruction bytes to know what they
/// are approving. Here the set of things governance *can* do is fixed at deploy
/// time and visible in the IDL, so a voter reads the variant and knows the blast
/// radius. Extending the set requires a program upgrade — which is itself
/// governed. The trade-off is deliberate: less general, far more auditable.
#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProposalAction {
    /// Signalling only. Records the outcome on chain and moves no funds.
    Signal,

    /// Transfer `amount` of the treasury's token to `destination`.
    TreasuryTransfer { destination: Pubkey, amount: u64 },

    /// Retune staking emissions.
    SetStakingRewardRate {
        new_rate: u64,
        reward_period_end: i64,
    },

    /// Commit treasury tokens to a linear vesting schedule.
    ///
    /// `stream_id` is deliberately absent: it must equal the treasury's current
    /// `stream_count` at execution time, which is not knowable when the proposal
    /// is written. It is supplied as an execution argument and validated by the
    /// treasury, so a caller cannot choose an arbitrary slot.
    CreateVestingStream {
        beneficiary: Pubkey,
        total_amount: u64,
        start_ts: i64,
        cliff_ts: i64,
        end_ts: i64,
    },

    /// Stop future accrual on a stream. Already-vested tokens stay claimable.
    RevokeVestingStream { stream_id: u64 },

    /// Adjust the treasury's per-epoch spend cap.
    SetTreasurySpendCap { new_cap: u64, epoch_duration: i64 },

    /// Hand treasury spending rights to a different governance executor.
    ///
    /// The migration path. Without this variant the treasury's
    /// `set_governance_executor` is unreachable, and a superseded governance
    /// program can only be replaced by upgrading the treasury program itself.
    SetGovernanceExecutor { new_executor: Pubkey },
}

#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProposalState {
    /// Created but not open for voting. The weight snapshot is not taken yet.
    Draft,
    Voting,
    /// Met quorum and approval, awaiting `queue`.
    Succeeded,
    Defeated,
    /// In the timelock. `eta` is set.
    Queued,
    Executed,
    /// Vetoed by the guardian.
    Cancelled,
}

impl ProposalState {
    /// Whether the guardian may still veto from this state.
    ///
    /// Everything up to and including `Queued`. Once executed the effect has
    /// already happened, and a "cancellation" of a completed action would be a
    /// lie about what the chain did.
    pub fn is_cancellable(self) -> bool {
        matches!(
            self,
            ProposalState::Draft
                | ProposalState::Voting
                | ProposalState::Succeeded
                | ProposalState::Queued
        )
    }

    /// Whether this is an end state.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            ProposalState::Executed | ProposalState::Defeated | ProposalState::Cancelled
        )
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, PartialEq, Eq, Debug)]
pub enum VoteChoice {
    For,
    Against,
    Abstain,
}

/// Governance configuration for one staking pool.
#[account]
#[derive(InitSpace, Debug)]
pub struct Realm {
    /// May change governance parameters. Intended end state is the realm's own
    /// executor PDA, so parameter changes go through the same vote as anything
    /// else.
    pub authority: Pubkey,

    /// May veto a proposal before execution, and may do nothing else. A guardian
    /// that could also *pass* proposals would not be a safety mechanism; it
    /// would be an admin key wearing a safety mechanism's name.
    pub guardian: Pubkey,

    /// The staking pool whose positions confer vote weight. A position from any
    /// other pool is rejected.
    pub staking_pool: Pubkey,

    /// Fraction of snapshotted weight that must vote (any choice) for the
    /// result to count.
    pub quorum_bps: u16,
    /// Fraction of decisive (for + against) weight that must be `For`.
    pub approval_bps: u16,

    pub voting_period: i64,
    pub timelock_delay: i64,

    /// Minimum weight a proposer must hold, to price out spam.
    pub min_weight_to_propose: u64,

    /// Monotonic counter seeding proposal PDAs.
    pub proposal_count: u64,

    pub bump: u8,
    /// Bump for the executor PDA, stored so only the canonical bump is used.
    pub executor_bump: u8,
}

#[account]
#[derive(InitSpace, Debug)]
pub struct Proposal {
    pub realm: Pubkey,
    pub proposer: Pubkey,
    pub id: u64,

    pub state: ProposalState,
    pub action: ProposalAction,

    #[max_len(MAX_TITLE_LEN)]
    pub title: String,
    /// Off-chain discussion / full text. On-chain storage is expensive and
    /// rationale is not something a program needs to read.
    #[max_len(MAX_URI_LEN)]
    pub descriptor_uri: String,

    pub created_at: i64,
    pub voting_starts_at: i64,
    pub voting_ends_at: i64,
    /// Earliest execution time. Zero until queued.
    pub eta: i64,

    pub for_votes: u64,
    pub against_votes: u64,
    pub abstain_votes: u64,

    /// `pool.total_weighted` at activation — the quorum denominator.
    ///
    /// Fixed at activation rather than read live at finalisation, so a whale
    /// cannot defeat a proposal by staking more (inflating the denominator)
    /// after seeing how the vote is going.
    pub total_weight_snapshot: u64,

    pub bump: u8,
}

impl Proposal {
    /// Total weight that participated, including abstentions.
    pub fn total_votes(&self) -> Result<u64> {
        self.for_votes
            .checked_add(self.against_votes)
            .and_then(|v| v.checked_add(self.abstain_votes))
            .ok_or_else(|| GovernanceError::MathOverflow.into())
    }

    /// Weight that expressed a preference. Abstentions count toward quorum but
    /// not toward approval — the standard Compound/OZ semantics, and the reason
    /// abstaining is a meaningful act rather than just not voting.
    pub fn decisive_votes(&self) -> Result<u64> {
        self.for_votes
            .checked_add(self.against_votes)
            .ok_or_else(|| GovernanceError::MathOverflow.into())
    }

    /// Whether enough weight participated for the result to count.
    pub fn quorum_reached(&self, quorum_bps: u16) -> Result<bool> {
        let participated = self.total_votes()? as u128;
        let required = (self.total_weight_snapshot as u128)
            .checked_mul(quorum_bps as u128)
            .ok_or(GovernanceError::MathOverflow)?;
        // Compare cross-multiplied to avoid dividing (and truncating) either side.
        Ok(participated
            .checked_mul(BPS_DENOMINATOR)
            .ok_or(GovernanceError::MathOverflow)?
            >= required)
    }

    /// Whether the `For` side cleared the approval threshold.
    pub fn approval_reached(&self, approval_bps: u16) -> Result<bool> {
        let decisive = self.decisive_votes()?;
        // With no decisive votes there is nothing to approve. Guarding here also
        // keeps the comparison below from treating 0 >= 0 as approval.
        if decisive == 0 {
            return Ok(false);
        }
        let required = (decisive as u128)
            .checked_mul(approval_bps as u128)
            .ok_or(GovernanceError::MathOverflow)?;
        Ok((self.for_votes as u128)
            .checked_mul(BPS_DENOMINATOR)
            .ok_or(GovernanceError::MathOverflow)?
            >= required)
    }

    /// The outcome this proposal would resolve to under `realm`'s thresholds.
    pub fn outcome(&self, quorum_bps: u16, approval_bps: u16) -> Result<ProposalState> {
        if self.quorum_reached(quorum_bps)? && self.approval_reached(approval_bps)? {
            Ok(ProposalState::Succeeded)
        } else {
            Ok(ProposalState::Defeated)
        }
    }

    /// Records a vote of `weight` for `choice`.
    pub fn tally(&mut self, choice: VoteChoice, weight: u64) -> Result<()> {
        let slot = match choice {
            VoteChoice::For => &mut self.for_votes,
            VoteChoice::Against => &mut self.against_votes,
            VoteChoice::Abstain => &mut self.abstain_votes,
        };
        *slot = slot
            .checked_add(weight)
            .ok_or(GovernanceError::MathOverflow)?;
        Ok(())
    }

    /// Last moment a queued proposal may be executed.
    pub fn expires_at(&self) -> Result<i64> {
        self.eta
            .checked_add(EXECUTION_GRACE_PERIOD)
            .ok_or_else(|| GovernanceError::MathOverflow.into())
    }

    /// Asserts the proposal is in `expected`.
    pub fn require_state(&self, expected: ProposalState) -> Result<()> {
        require!(
            self.state == expected,
            GovernanceError::InvalidProposalState
        );
        Ok(())
    }
}

/// One position's vote on one proposal. PDA: `["vote", proposal, position]`.
///
/// Seeding by *position* rather than by wallet is what lets a holder with three
/// positions vote their full weight while keeping each position's vote exactly
/// once-only. Double voting is an `init` constraint failure, not a runtime check.
#[account]
#[derive(InitSpace, Debug)]
pub struct VoteRecord {
    pub proposal: Pubkey,
    pub position: Pubkey,
    pub voter: Pubkey,
    pub choice: VoteChoice,
    /// Weight counted, retained so the tally can be audited after the fact.
    pub weight: u64,
    pub voted_at: i64,
    pub bump: u8,
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;

    fn proposal(snapshot: u64) -> Proposal {
        Proposal {
            realm: Pubkey::default(),
            proposer: Pubkey::default(),
            id: 0,
            state: ProposalState::Voting,
            action: ProposalAction::Signal,
            title: String::new(),
            descriptor_uri: String::new(),
            created_at: 0,
            voting_starts_at: 0,
            voting_ends_at: 3 * DAY,
            eta: 0,
            for_votes: 0,
            against_votes: 0,
            abstain_votes: 0,
            total_weight_snapshot: snapshot,
            bump: 255,
        }
    }

    // ---------------------------------------------------------------- quorum

    #[test]
    fn quorum_counts_abstentions() {
        let mut p = proposal(1_000);
        // 20% quorum on a 1000 snapshot needs 200 of participation.
        p.for_votes = 100;
        p.abstain_votes = 100;
        assert!(p.quorum_reached(2_000).unwrap());

        // Abstentions are the only reason this reaches quorum; without them it
        // would not.
        p.abstain_votes = 0;
        assert!(!p.quorum_reached(2_000).unwrap());
    }

    #[test]
    fn quorum_boundary_is_inclusive() {
        let mut p = proposal(1_000);
        p.for_votes = 199;
        assert!(!p.quorum_reached(2_000).unwrap());
        p.for_votes = 200;
        assert!(p.quorum_reached(2_000).unwrap());
    }

    #[test]
    fn quorum_is_not_lost_to_rounding() {
        // 3333 bps of 7 is 2.33; cross-multiplication means 3 participating
        // weight clears it and 2 does not, with no truncation in between.
        let mut p = proposal(7);
        p.for_votes = 2;
        assert!(!p.quorum_reached(3_333).unwrap());
        p.for_votes = 3;
        assert!(p.quorum_reached(3_333).unwrap());
    }

    // -------------------------------------------------------------- approval

    #[test]
    fn abstentions_do_not_help_approval() {
        let mut p = proposal(1_000);
        p.for_votes = 100;
        p.against_votes = 100;
        p.abstain_votes = 1_000_000;

        // A dead heat is not a majority, however many abstentions surround it.
        assert!(!p.approval_reached(5_001).unwrap());
    }

    #[test]
    fn simple_majority_threshold() {
        let mut p = proposal(1_000);
        p.for_votes = 501;
        p.against_votes = 499;
        assert!(p.approval_reached(5_001).unwrap());

        p.for_votes = 500;
        p.against_votes = 500;
        assert!(!p.approval_reached(5_001).unwrap());
    }

    #[test]
    fn supermajority_threshold() {
        let mut p = proposal(1_000);
        // 2/3 threshold: 667 of 1000 decisive passes, 666 does not.
        p.for_votes = 667;
        p.against_votes = 333;
        assert!(p.approval_reached(6_667).unwrap());

        p.for_votes = 666;
        p.against_votes = 334;
        assert!(!p.approval_reached(6_667).unwrap());
    }

    #[test]
    fn no_decisive_votes_is_not_approval() {
        let mut p = proposal(1_000);
        p.abstain_votes = 1_000;
        // Everyone abstained: quorum met, but nothing was approved.
        assert!(p.quorum_reached(5_000).unwrap());
        assert!(!p.approval_reached(5_001).unwrap());
        assert_eq!(p.outcome(5_000, 5_001).unwrap(), ProposalState::Defeated);
    }

    #[test]
    fn unanimous_but_below_quorum_is_defeated() {
        let mut p = proposal(1_000_000);
        p.for_votes = 10; // unanimous, but nobody showed up
        assert!(p.approval_reached(5_001).unwrap());
        assert!(!p.quorum_reached(2_000).unwrap());
        assert_eq!(p.outcome(2_000, 5_001).unwrap(), ProposalState::Defeated);
    }

    #[test]
    fn quorum_and_approval_together_succeed() {
        let mut p = proposal(1_000);
        p.for_votes = 400;
        p.against_votes = 100;
        assert_eq!(p.outcome(2_000, 5_001).unwrap(), ProposalState::Succeeded);
    }

    // ----------------------------------------------------------------- tally

    #[test]
    fn tally_routes_each_choice() {
        let mut p = proposal(1_000);
        p.tally(VoteChoice::For, 10).unwrap();
        p.tally(VoteChoice::For, 5).unwrap();
        p.tally(VoteChoice::Against, 3).unwrap();
        p.tally(VoteChoice::Abstain, 2).unwrap();

        assert_eq!(p.for_votes, 15);
        assert_eq!(p.against_votes, 3);
        assert_eq!(p.abstain_votes, 2);
        assert_eq!(p.total_votes().unwrap(), 20);
        assert_eq!(p.decisive_votes().unwrap(), 18);
    }

    #[test]
    fn tally_rejects_overflow() {
        let mut p = proposal(u64::MAX);
        p.tally(VoteChoice::For, u64::MAX).unwrap();
        assert!(p.tally(VoteChoice::For, 1).is_err());
    }

    // ------------------------------------------------------------- lifecycle

    #[test]
    fn guardian_can_veto_until_executed() {
        assert!(ProposalState::Draft.is_cancellable());
        assert!(ProposalState::Voting.is_cancellable());
        assert!(ProposalState::Succeeded.is_cancellable());
        assert!(ProposalState::Queued.is_cancellable());

        // Once the effect has happened, "cancelling" it would misreport history.
        assert!(!ProposalState::Executed.is_cancellable());
        assert!(!ProposalState::Defeated.is_cancellable());
        assert!(!ProposalState::Cancelled.is_cancellable());
    }

    #[test]
    fn terminal_states_are_terminal() {
        assert!(ProposalState::Executed.is_terminal());
        assert!(ProposalState::Defeated.is_terminal());
        assert!(ProposalState::Cancelled.is_terminal());

        assert!(!ProposalState::Draft.is_terminal());
        assert!(!ProposalState::Voting.is_terminal());
        assert!(!ProposalState::Succeeded.is_terminal());
        assert!(!ProposalState::Queued.is_terminal());
    }

    #[test]
    fn require_state_gates_transitions() {
        let p = proposal(1_000);
        assert!(p.require_state(ProposalState::Voting).is_ok());
        assert!(p.require_state(ProposalState::Queued).is_err());
    }

    #[test]
    fn queued_proposals_expire() {
        let mut p = proposal(1_000);
        p.eta = 1_000;
        assert_eq!(p.expires_at().unwrap(), 1_000 + EXECUTION_GRACE_PERIOD);
    }

    #[test]
    fn expiry_cannot_overflow() {
        let mut p = proposal(1_000);
        p.eta = i64::MAX;
        assert!(p.expires_at().is_err());
    }
}
