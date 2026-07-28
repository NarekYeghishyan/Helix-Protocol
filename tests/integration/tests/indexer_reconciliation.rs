//! The indexer's view of the protocol, checked against the chain's.
//!
//! Every analytics stack claims its numbers match on-chain state. Almost none of
//! them can demonstrate it, because doing so needs both halves running against
//! each other — and by the time both exist, nobody goes back and writes the test.
//!
//! These run real transactions against the real BPF programs, capture the logs
//! the runtime produced, fold them through [`helix_indexer`], and compare the
//! result to the accounts those same transactions wrote. Nothing is stubbed: the
//! logs are the runtime's, the decoding is the indexer's, and the expected values
//! are read back out of the accounts rather than restated.
//!
//! What this can and cannot show: it proves the decode-and-fold path agrees with
//! the chain over the flows exercised here. It says nothing about RPC delivery,
//! reorgs, or backfill — none of which exist yet, and all of which are Phase 4
//! remainder. See [`docs/ROADMAP.md`](../../../docs/ROADMAP.md).

use helix_governance::state::{ProposalAction, ProposalState, VoteChoice};
use helix_indexer::projection::Analytics;
use helix_indexer::{parse, Program};
use helix_integration_tests::bootstrap::HOUR;
use helix_integration_tests::{pda, System, TransferFee};
use helix_staking::state::{LockTier, Pool, Position};
use helix_treasury::state::Treasury;
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_signer::Signer as _;

use anchor_lang::prelude::Pubkey;

/// A `System` whose every transaction is also fed to an indexer.
///
/// The indexer sees exactly what a real one would: the log lines, in order, with
/// nothing else passed alongside them.
struct Indexed {
    sys: System,
    analytics: Analytics,
    /// Distinct per transaction, standing in for a signature. The real ones are
    /// available but a counter makes a failure readable.
    next_signature: usize,
    /// Anomalies seen across the whole run. Must stay empty: every transaction
    /// here is small, so any truncation or undecodable payload is a defect.
    anomalies: Vec<String>,
}

impl Indexed {
    fn new(fee: Option<TransferFee>, treasury_funding: u64) -> Self {
        // Bootstrap runs before the indexer is attached, exactly as a real one
        // deployed after genesis would miss it. Everything the tests assert on
        // is emitted afterwards.
        Self {
            sys: System::bootstrap(fee, treasury_funding),
            analytics: Analytics::new(),
            next_signature: 0,
            anomalies: Vec::new(),
        }
    }

    /// Sends an instruction and indexes whatever it emitted.
    fn send(&mut self, ix: Instruction, signers: &[&Keypair]) {
        let meta = self.sys.env.send_metered(&[ix], signers);
        self.ingest(&meta.logs);
    }

    fn ingest(&mut self, logs: &[String]) {
        let parsed = parse(logs);
        for anomaly in &parsed.anomalies {
            self.anomalies.push(format!("{anomaly:?}"));
        }
        let signature = format!("sig-{}", self.next_signature);
        self.next_signature += 1;
        self.analytics.apply_transaction(&signature, &parsed.events);
    }

    /// Replays every transaction seen so far a second time.
    ///
    /// A real indexer gets this for free: confirmed logs are redelivered, and a
    /// backfill run over a range the live stream already covered replays all of
    /// it. If that double-counts, every figure is wrong and nothing says so.
    fn replay_last(&mut self, logs: &[String], signature: &str) -> usize {
        let parsed = parse(logs);
        self.analytics.apply_transaction(signature, &parsed.events)
    }

    fn assert_no_anomalies(&self) {
        assert!(
            self.anomalies.is_empty(),
            "the indexer could not read part of the log: {:?}",
            self.anomalies
        );
    }
}

/// Reads the pool account and asserts the projection matches it field by field.
fn assert_pool_matches(indexed: &Indexed) {
    let chain: Pool = indexed.sys.env.anchor_account(&indexed.sys.pool);
    let indexed_pool = indexed
        .analytics
        .pools
        .get(&indexed.sys.pool)
        .unwrap_or_else(|| panic!("indexer never saw pool {}", indexed.sys.pool));

    assert_eq!(
        indexed_pool.total_staked, chain.total_staked,
        "total_staked disagrees"
    );
    assert_eq!(
        indexed_pool.total_weighted, chain.total_weighted,
        "total_weighted disagrees"
    );
    assert_eq!(
        indexed_pool.position_count, chain.position_count,
        "position_count disagrees"
    );
    assert_eq!(
        indexed_pool.total_rewards_funded, chain.total_rewards_funded,
        "total_rewards_funded disagrees"
    );
    assert_eq!(
        indexed_pool.total_rewards_paid, chain.total_rewards_paid,
        "total_rewards_paid disagrees"
    );
    assert_eq!(
        indexed_pool.reward_rate, chain.reward_rate,
        "reward_rate disagrees"
    );
    assert_eq!(indexed_pool.paused, chain.paused, "paused disagrees");
}

fn assert_position_matches(indexed: &Indexed, position: &Pubkey) {
    let chain: Position = indexed.sys.env.anchor_account(position);
    let indexed_position = indexed
        .analytics
        .positions
        .get(position)
        .unwrap_or_else(|| panic!("indexer never saw position {position}"));

    assert_eq!(indexed_position.amount, chain.amount, "amount disagrees");
    assert_eq!(
        indexed_position.weighted_amount, chain.weighted_amount,
        "weighted_amount disagrees"
    );
    assert_eq!(indexed_position.owner, chain.owner, "owner disagrees");
    assert_eq!(indexed_position.tier, chain.tier, "tier disagrees");
    assert_eq!(
        indexed_position.lock_end, chain.lock_end,
        "lock_end disagrees"
    );
}

const STAKE: u64 = 1_000_000_000;
const REWARD_FUNDING: u64 = 200_000_000_000;
const REWARD_RATE: u64 = 1_000_000;

/// Drives a pool through its whole lifecycle and compares the two views at the
/// end of every step.
#[test]
fn the_indexed_pool_matches_the_chain_through_a_full_staking_lifecycle() {
    let mut indexed = Indexed::new(None, 0);

    let ix = indexed.sys.fund_rewards_ix(REWARD_FUNDING);
    indexed.send(ix, &[]);
    assert_pool_matches(&indexed);

    let period_end = indexed.sys.env.now() + 100_000;
    let ix = indexed.sys.set_reward_rate_ix(REWARD_RATE, period_end);
    indexed.send(ix, &[]);
    assert_pool_matches(&indexed);

    // Three independent stakers, so the projection has to aggregate rather than
    // mirror a single position.
    let mut stakers = Vec::new();
    for _ in 0..3 {
        let owner = Keypair::new();
        let (tokens, position_id) = indexed.sys.prepare_staker(&owner.pubkey(), STAKE);
        let (position, _) = pda::position(&indexed.sys.pool, &owner.pubkey(), position_id);
        let ix =
            indexed
                .sys
                .stake_ix_for(&owner.pubkey(), &tokens, position_id, STAKE, LockTier::Gold);
        indexed.send(ix, &[&owner]);
        assert_pool_matches(&indexed);
        assert_position_matches(&indexed, &position);
        stakers.push((owner, tokens, position));
    }

    assert_eq!(indexed.analytics.tvl(&indexed.sys.pool), 3 * STAKE);
    assert_eq!(
        indexed
            .analytics
            .staker_distribution(&indexed.sys.pool)
            .len(),
        3
    );

    // Accrue, then claim on one position.
    indexed.sys.env.warp_forward(HOUR);
    let (owner, tokens, position) = &stakers[0];
    let ix = indexed.sys.claim_ix_for(&owner.pubkey(), tokens, *position);
    let owner = owner.insecure_clone();
    indexed.send(ix, &[&owner]);
    assert_pool_matches(&indexed);
    assert_position_matches(&indexed, position);

    // Unlock and partially withdraw. This is the step that needed `Unstaked` to
    // carry the post-withdrawal weight: without it, `total_weighted` can only be
    // reconstructed by re-running the tier table off chain.
    indexed.sys.env.warp_forward(LockTier::Gold.duration() + 1);
    let ix = indexed
        .sys
        .unstake_ix_for(&owner.pubkey(), tokens, *position, STAKE / 4);
    indexed.send(ix, &[&owner]);
    assert_pool_matches(&indexed);
    assert_position_matches(&indexed, position);

    // And fully.
    let ix = indexed
        .sys
        .unstake_ix_for(&owner.pubkey(), tokens, *position, STAKE - STAKE / 4);
    indexed.send(ix, &[&owner]);
    assert_pool_matches(&indexed);
    assert_position_matches(&indexed, position);

    assert_eq!(indexed.analytics.tvl(&indexed.sys.pool), 2 * STAKE);
    assert_eq!(
        indexed
            .analytics
            .staker_distribution(&indexed.sys.pool)
            .len(),
        2,
        "a fully withdrawn position is not a staker"
    );
    indexed.assert_no_anomalies();
}

/// The event stream must be right about fees too.
///
/// On a fee-bearing mint the amount sent is not the amount credited, and an
/// indexer reading `amount_sent` overstates TVL by the fee on every deposit —
/// invisibly, because the two figures are identical on a plain mint.
#[test]
fn indexed_tvl_follows_credited_amounts_on_a_fee_bearing_mint() {
    let fee = TransferFee {
        basis_points: 100, // 1%
        maximum_fee: u64::MAX,
    };
    let mut indexed = Indexed::new(Some(fee), 0);

    let owner = Keypair::new();
    let (tokens, position_id) = indexed.sys.prepare_staker(&owner.pubkey(), STAKE);
    let (position, _) = pda::position(&indexed.sys.pool, &owner.pubkey(), position_id);
    let ix = indexed.sys.stake_ix_for(
        &owner.pubkey(),
        &tokens,
        position_id,
        STAKE,
        LockTier::Bronze,
    );
    indexed.send(ix, &[&owner]);

    let credited = STAKE - fee.expected_on(STAKE);
    assert_eq!(
        indexed.analytics.tvl(&indexed.sys.pool),
        credited,
        "TVL followed the amount sent rather than the amount credited"
    );
    assert_pool_matches(&indexed);
    assert_position_matches(&indexed, &position);
    indexed.assert_no_anomalies();
}

/// The governance lifecycle, including the nested-CPI execution.
#[test]
fn the_indexed_proposal_matches_the_chain_through_execution() {
    const FUNDING: u64 = 5_000_000;
    // Funded after the indexer is attached, so it sees the `Deposited` event.
    // `partial_history_is_reported_rather_than_silently_wrong` covers what
    // happens when it does not.
    let mut indexed = Indexed::new(None, 0);
    let ix = indexed.sys.fund_treasury_ix(FUNDING);
    indexed.send(ix, &[]);

    indexed.sys.fund_voter(STAKE);
    let voter = indexed.sys.voter.insecure_clone();
    let ix = indexed.sys.stake_ix(0, STAKE, LockTier::Gold);
    indexed.send(ix, &[&voter]);

    let (position, _) = pda::position(&indexed.sys.pool, &voter.pubkey(), 0);
    let destination = indexed
        .sys
        .new_token_account(&indexed.sys.env.payer_pubkey());
    let (proposal, _) = pda::proposal(&indexed.sys.realm, 0);
    let spend = FUNDING / 2;

    let ix = indexed.sys.create_proposal_ix(
        0,
        ProposalAction::TreasuryTransfer {
            destination,
            amount: spend,
        },
        position,
    );
    indexed.send(ix, &[&voter]);
    assert_eq!(
        indexed.analytics.proposals[&proposal].state,
        ProposalState::Draft
    );

    let ix = indexed.sys.activate_ix(proposal);
    indexed.send(ix, &[]);
    assert_eq!(
        indexed.analytics.proposals[&proposal].state,
        ProposalState::Voting
    );

    let ix = indexed.sys.vote_ix(proposal, position, VoteChoice::For);
    indexed.send(ix, &[&voter]);

    indexed.sys.env.warp_forward(HOUR + 1);
    let ix = indexed.sys.advance_ix(proposal, true);
    indexed.send(ix, &[]);
    let ix = indexed.sys.advance_ix(proposal, false);
    indexed.send(ix, &[]);
    indexed.sys.env.warp_forward(HOUR + 1);

    let chain: helix_governance::state::Proposal = indexed.sys.env.anchor_account(&proposal);
    let view = &indexed.analytics.proposals[&proposal];
    assert_eq!(view.state, ProposalState::Queued);
    assert_eq!(view.for_votes, chain.for_votes, "for_votes disagrees");
    assert_eq!(view.against_votes, chain.against_votes);
    assert_eq!(view.abstain_votes, chain.abstain_votes);
    assert_eq!(
        view.total_weight_snapshot, chain.total_weight_snapshot,
        "the quorum denominator disagrees"
    );
    assert!(view.voters.contains(&voter.pubkey()));

    // The nested-CPI case: governance at depth 1 emits ProposalExecuted, the
    // treasury at depth 2 emits Spent, and both land after a deeper program has
    // already returned.
    let ix = indexed
        .sys
        .execute_treasury_transfer_ix(proposal, destination);
    indexed.send(ix, &[]);

    let view = &indexed.analytics.proposals[&proposal];
    assert_eq!(view.state, ProposalState::Executed);

    let chain: Treasury = indexed.sys.env.anchor_account(&indexed.sys.treasury);
    let treasury = &indexed.analytics.treasuries[&indexed.sys.treasury];
    assert_eq!(
        treasury.total_spent, chain.total_spent,
        "the treasury spend was attributed to the wrong program, or lost"
    );
    assert_eq!(
        indexed.analytics.treasury_balance(&indexed.sys.treasury),
        FUNDING - spend
    );
    indexed.assert_no_anomalies();
}

/// Both events of the deepest call stack must be attributed to their own
/// programs, not to the transaction's outermost one.
#[test]
fn a_nested_cpi_attributes_each_event_to_its_own_program() {
    const FUNDING: u64 = 5_000_000;
    let mut sys = System::bootstrap(None, FUNDING);
    sys.fund_voter(STAKE);
    let position = sys.stake(0, STAKE, LockTier::Gold);
    let destination = sys.new_token_account(&sys.env.payer_pubkey());
    let proposal = sys.pass_treasury_transfer(0, position, destination, FUNDING / 2);

    let ix = sys.execute_treasury_transfer_ix(proposal, destination);
    let meta = sys.env.send_metered(&[ix], &[]);
    let parsed = parse(&meta.logs);

    assert!(parsed.is_complete(), "anomalies: {:?}", parsed.anomalies);
    assert_eq!(parsed.events_of(Program::Treasury).count(), 1);
    assert_eq!(parsed.events_of(Program::Governance).count(), 1);

    let treasury_event = parsed.events_of(Program::Treasury).next().unwrap();
    let governance_event = parsed.events_of(Program::Governance).next().unwrap();
    assert_eq!(
        treasury_event.depth, 2,
        "the treasury's event was emitted under governance"
    );
    assert_eq!(governance_event.depth, 1);
    assert!(
        treasury_event.log_index < governance_event.log_index,
        "the inner program's event should be logged first"
    );
}

/// Redelivering a transaction must not move a single number.
#[test]
fn replaying_a_transaction_double_counts_nothing() {
    let mut indexed = Indexed::new(None, 0);

    let ix = indexed.sys.fund_rewards_ix(REWARD_FUNDING);
    indexed.send(ix, &[]);

    let owner = Keypair::new();
    let (tokens, position_id) = indexed.sys.prepare_staker(&owner.pubkey(), STAKE);
    let ix = indexed.sys.stake_ix_for(
        &owner.pubkey(),
        &tokens,
        position_id,
        STAKE,
        LockTier::Silver,
    );

    // Capture the logs so the same ones can be fed in twice, which is precisely
    // what a backfill overlapping a live subscription does.
    let meta = indexed.sys.env.send_metered(&[ix], &[&owner]);
    indexed.ingest(&meta.logs);

    let before = indexed.analytics.pools[&indexed.sys.pool].clone();
    let applied = indexed.analytics.applied_count();

    let new_events = indexed.replay_last(&meta.logs, "sig-1");
    assert_eq!(new_events, 0, "a replay was treated as new events");
    assert_eq!(indexed.analytics.pools[&indexed.sys.pool], before);
    assert_eq!(indexed.analytics.applied_count(), applied);

    assert_pool_matches(&indexed);
    indexed.assert_no_anomalies();
}

/// An indexer that starts mid-history says so.
///
/// This began as a bug in the test above: the treasury was funded during
/// bootstrap, before the indexer was attached, so it never saw the deposit and
/// `treasury_balance` returned 0 while the chain held 2.5M. Nothing flagged it.
/// A dashboard would have shown an empty treasury with complete confidence.
///
/// The projection now records the first event that materialises an entity it
/// never saw created. The numbers are still the best available — the events carry
/// running totals rather than deltas, so one deposit re-synchronises the figure —
/// but "best available" and "complete" are different claims, and a caller is
/// entitled to know which one it is holding.
#[test]
fn partial_history_is_reported_rather_than_silently_wrong() {
    const FUNDING: u64 = 5_000_000;
    // Funded during bootstrap: the indexer attaches afterwards and misses it,
    // exactly as one deployed after genesis would.
    let mut indexed = Indexed::new(None, FUNDING);
    assert!(indexed.analytics.orphaned.is_empty(), "nothing seen yet");

    let owner = Keypair::new();
    let (tokens, position_id) = indexed.sys.prepare_staker(&owner.pubkey(), STAKE);
    let ix = indexed.sys.stake_ix_for(
        &owner.pubkey(),
        &tokens,
        position_id,
        STAKE,
        LockTier::Bronze,
    );
    indexed.send(ix, &[&owner]);

    // The pool was initialised before the indexer existed, so the first event
    // touching it is flagged.
    assert_eq!(
        indexed.analytics.orphaned.len(),
        1,
        "a pool materialised from a Staked event should be reported as partial"
    );
    // The log itself was read completely; the gap is in history, not in parsing.
    indexed.assert_no_anomalies();

    // And the figures that only need the events seen are still exactly right.
    assert_pool_matches(&indexed);

    // A single deposit re-synchronises the treasury, because the event carries a
    // running total rather than a delta.
    let ix = indexed.sys.fund_treasury_ix(1_000);
    indexed.send(ix, &[]);
    let chain: Treasury = indexed.sys.env.anchor_account(&indexed.sys.treasury);
    assert_eq!(
        indexed.analytics.treasuries[&indexed.sys.treasury].total_deposited, chain.total_deposited,
        "a running total should recover the figure despite the missed history"
    );
}

/// Every event the staking program emits must decode.
///
/// A discriminator the indexer does not recognise is reported rather than
/// skipped, so this fails loudly if a program gains an event and the indexer is
/// not extended — the failure mode being guarded against is an indexer that is
/// quietly older than the chain it is reading.
#[test]
fn no_emitted_event_goes_unrecognised() {
    let mut indexed = Indexed::new(None, 1_000_000);

    let ix = indexed.sys.fund_rewards_ix(REWARD_FUNDING);
    indexed.send(ix, &[]);
    let period_end = indexed.sys.env.now() + 100_000;
    let ix = indexed.sys.set_reward_rate_ix(REWARD_RATE, period_end);
    indexed.send(ix, &[]);

    indexed.sys.fund_voter(STAKE);
    let voter = indexed.sys.voter.insecure_clone();
    let ix = indexed.sys.stake_ix(0, STAKE, LockTier::Flexible);
    indexed.send(ix, &[&voter]);

    let ix = indexed.sys.fund_treasury_ix(1_000);
    indexed.send(ix, &[]);

    indexed.sys.env.warp_forward(HOUR);
    let (position, _) = pda::position(&indexed.sys.pool, &voter.pubkey(), 0);
    let ix = indexed.sys.claim_ix(position);
    indexed.send(ix, &[&voter]);

    let ix = indexed.sys.unstake_ix(position, STAKE);
    indexed.send(ix, &[&voter]);

    // No anomalies means every `Program data:` line decoded into a known event.
    // Orphans are a separate matter and are expected here: the pool and treasury
    // were created during bootstrap, before the indexer was attached.
    indexed.assert_no_anomalies();
    assert!(indexed.analytics.applied_count() >= 6);
}
