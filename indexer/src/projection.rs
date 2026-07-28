//! Folding the event stream back into protocol state.
//!
//! This is the part worth being suspicious of. Every analytics stack claims its
//! numbers match the chain; almost none of them prove it. The whole projection is
//! therefore built so it *can* be checked — each field here has a counterpart in
//! an on-chain account, and `indexer_reconciliation.rs` runs real transactions,
//! decodes their real logs, and asserts the two agree field by field.
//!
//! Where a figure has to be derived rather than read from an event, the
//! derivation calls the program's own code (`LockTier::apply_weight`) rather than
//! a copy of it. A reimplementation is a second source of truth, and the two
//! diverge silently at the first program upgrade.

use anchor_lang::prelude::Pubkey;
use helix_governance::state::ProposalState;
use helix_staking::state::LockTier;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use crate::event::HelixEvent;
use crate::logs::EmittedEvent;

/// Identifies an event uniquely across the whole chain.
///
/// A transaction signature is not enough on its own: one transaction emits
/// several events, and two of them can be byte-identical. Pairing the signature
/// with the log line makes replaying a transaction — which happens routinely,
/// because confirmed logs can be delivered more than once and re-fetched after a
/// reorg — a no-op instead of a double count.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct EventId {
    pub signature: String,
    pub log_index: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PoolStats {
    pub authority: Option<Pubkey>,
    pub total_staked: u64,
    pub total_weighted: u64,
    pub position_count: u64,
    pub reward_rate: u64,
    pub reward_period_end: i64,
    pub total_rewards_funded: u64,
    pub total_rewards_paid: u64,
    pub paused: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PositionStats {
    pub pool: Pubkey,
    pub owner: Pubkey,
    pub position_id: u64,
    pub amount: u64,
    pub weighted_amount: u64,
    pub tier: LockTier,
    pub lock_end: i64,
    pub rewards_claimed: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProposalStats {
    pub realm: Pubkey,
    pub proposer: Pubkey,
    pub id: u64,
    pub title: String,
    pub state: ProposalState,
    pub for_votes: u64,
    pub against_votes: u64,
    pub abstain_votes: u64,
    pub total_weight_snapshot: u64,
    /// How many positions the snapshot above covers. Tracked because a consumer
    /// checking whether a vote came from the electorate needs both halves, and
    /// because the chain stores both — a projection that dropped it would
    /// disagree with the account it claims to mirror.
    pub position_count_snapshot: u64,
    pub voters: BTreeSet<Pubkey>,
    pub eta: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TreasuryStats {
    pub governance_executor: Option<Pubkey>,
    pub total_deposited: u64,
    pub total_spent: u64,
    pub epoch_spend_cap: u64,
    /// Streams that have not been revoked, by stream account.
    pub open_streams: BTreeMap<Pubkey, StreamStats>,
    pub total_stream_claims: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StreamStats {
    pub beneficiary: Pubkey,
    pub stream_id: u64,
    pub total_amount: u64,
    pub claimed: u64,
    pub revoked: bool,
}

/// Protocol state as reconstructed from events.
#[derive(Clone, Debug, Default)]
pub struct Analytics {
    pub pools: BTreeMap<Pubkey, PoolStats>,
    pub positions: BTreeMap<Pubkey, PositionStats>,
    pub proposals: BTreeMap<Pubkey, ProposalStats>,
    pub treasuries: BTreeMap<Pubkey, TreasuryStats>,
    /// Every event already folded in, so a redelivery changes nothing.
    applied: BTreeSet<EventId>,
    /// Events that referenced an entity never seen created.
    ///
    /// A set rather than a list: one event can touch several unknowns — a
    /// `RewardsClaimed` for a position *and* a pool the stream never saw — and
    /// that is one gap in history, not two. On a backfill starting mid-history
    /// these are expected; on a live stream they mean something was dropped.
    pub orphaned: BTreeSet<EventId>,
}

impl Analytics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn applied_count(&self) -> usize {
        self.applied.len()
    }

    /// Folds in every event of one transaction.
    ///
    /// Returns how many were new. Calling this twice with the same transaction
    /// returns zero the second time and leaves the state untouched, which is the
    /// property the whole ingestion path depends on.
    pub fn apply_transaction(&mut self, signature: &str, events: &[EmittedEvent]) -> usize {
        events
            .iter()
            .filter(|emitted| {
                self.apply(
                    EventId {
                        signature: signature.to_owned(),
                        log_index: emitted.log_index,
                    },
                    &emitted.event,
                )
            })
            .count()
    }

    /// Folds in a single event. Returns false if it had already been applied.
    pub fn apply(&mut self, id: EventId, event: &HelixEvent) -> bool {
        if !self.applied.insert(id.clone()) {
            return false;
        }
        self.fold(id, event);
        true
    }

    /// The pool's row, noting it if this event is the first thing that ever
    /// mentioned it.
    ///
    /// Materialising a row from, say, a `Staked` event means the `PoolInitialized`
    /// that created it was never seen — the stream starts mid-history. The
    /// figures that follow are still the best available, because the events carry
    /// running totals rather than deltas, but "best available" and "complete" are
    /// different claims and the caller is entitled to know which one it has.
    fn pool_mut(&mut self, pool: Pubkey, id: &EventId) -> &mut PoolStats {
        if !self.pools.contains_key(&pool) {
            self.orphaned.insert(id.clone());
        }
        self.pools.entry(pool).or_default()
    }

    fn treasury_mut(&mut self, treasury: Pubkey, id: &EventId) -> &mut TreasuryStats {
        if !self.treasuries.contains_key(&treasury) {
            self.orphaned.insert(id.clone());
        }
        self.treasuries.entry(treasury).or_default()
    }

    fn fold(&mut self, id: EventId, event: &HelixEvent) {
        use HelixEvent as E;
        match event {
            // ------------------------------------------------------- staking
            E::PoolInitialized(e) => {
                self.pools.entry(e.pool).or_default().authority = Some(e.authority);
            }
            E::Staked(e) => {
                self.positions.insert(
                    e.position,
                    PositionStats {
                        pool: e.pool,
                        owner: e.owner,
                        position_id: e.position_id,
                        // The credited figure, never `amount_sent`. They differ
                        // on a mint with a Token-2022 transfer fee, and the
                        // credited one is what the position was built from.
                        amount: e.amount_credited,
                        weighted_amount: e.weighted_amount,
                        tier: e.tier,
                        lock_end: e.lock_end,
                        rewards_claimed: 0,
                    },
                );
                let pool = self.pool_mut(e.pool, &id);
                pool.total_staked = pool.total_staked.saturating_add(e.amount_credited);
                pool.total_weighted = pool.total_weighted.saturating_add(e.weighted_amount);
                pool.position_count = pool.position_count.saturating_add(1);
            }
            E::Unstaked(e) => {
                let Some(position) = self.positions.get_mut(&e.position) else {
                    self.orphaned.insert(id);
                    return;
                };
                let old_weighted = position.weighted_amount;
                position.amount = e.remaining;
                // Read from the event, never recomputed as
                // `tier.apply_weight(remaining)`. That recomputation is a second
                // implementation of the weight table, and it agrees with the
                // program right up until the table changes.
                position.weighted_amount = e.weighted_amount;

                let pool = self.pool_mut(e.pool, &id);
                pool.total_staked = pool.total_staked.saturating_sub(e.amount);
                pool.total_weighted = pool
                    .total_weighted
                    .saturating_sub(old_weighted.saturating_sub(e.weighted_amount));
            }
            E::RewardsClaimed(e) => {
                match self.positions.get_mut(&e.position) {
                    Some(position) => {
                        position.rewards_claimed = position.rewards_claimed.saturating_add(e.amount)
                    }
                    None => {
                        self.orphaned.insert(id.clone());
                    }
                }
                let pool = self.pool_mut(e.pool, &id);
                pool.total_rewards_paid = pool.total_rewards_paid.saturating_add(e.amount);
            }
            E::RewardsFunded(e) => {
                self.pool_mut(e.pool, &id).total_rewards_funded = e.total_funded;
            }
            E::RewardRateChanged(e) => {
                let pool = self.pool_mut(e.pool, &id);
                pool.reward_rate = e.new_rate;
                pool.reward_period_end = e.reward_period_end;
            }
            E::PoolPauseToggled(e) => {
                self.pool_mut(e.pool, &id).paused = e.paused;
            }
            E::AuthorityTransferAccepted(e) => {
                self.pool_mut(e.pool, &id).authority = Some(e.new_authority);
            }
            E::AuthorityTransferProposed(_) => {}

            // ---------------------------------------------------- governance
            E::ProposalCreated(e) => {
                self.proposals.insert(
                    e.proposal,
                    ProposalStats {
                        realm: e.realm,
                        proposer: e.proposer,
                        id: e.id,
                        title: e.title.clone(),
                        state: ProposalState::Draft,
                        for_votes: 0,
                        against_votes: 0,
                        abstain_votes: 0,
                        total_weight_snapshot: 0,
                        position_count_snapshot: 0,
                        voters: BTreeSet::new(),
                        eta: None,
                    },
                );
            }
            E::ProposalActivated(e) => {
                let Some(proposal) = self.proposals.get_mut(&e.proposal) else {
                    self.orphaned.insert(id);
                    return;
                };
                proposal.state = ProposalState::Voting;
                proposal.total_weight_snapshot = e.total_weight_snapshot;
                proposal.position_count_snapshot = e.position_count_snapshot;
            }
            E::VoteCast(e) => {
                let Some(proposal) = self.proposals.get_mut(&e.proposal) else {
                    self.orphaned.insert(id);
                    return;
                };
                // Taken from the running tallies the event carries rather than
                // accumulated here. The chain already did this arithmetic; doing
                // it again is a chance to disagree with it.
                proposal.for_votes = e.for_votes;
                proposal.against_votes = e.against_votes;
                proposal.abstain_votes = e.abstain_votes;
                // The individual choice belongs on the vote record, not here —
                // this row is the tally, and the per-vote detail is a separate
                // table (see `sql/schema.sql`).
                proposal.voters.insert(e.voter);
            }
            E::ProposalFinalized(e) => {
                let Some(proposal) = self.proposals.get_mut(&e.proposal) else {
                    self.orphaned.insert(id);
                    return;
                };
                proposal.state = e.outcome;
                proposal.for_votes = e.for_votes;
                proposal.against_votes = e.against_votes;
                proposal.abstain_votes = e.abstain_votes;
                proposal.total_weight_snapshot = e.total_weight_snapshot;
            }
            E::ProposalQueued(e) => {
                let Some(proposal) = self.proposals.get_mut(&e.proposal) else {
                    self.orphaned.insert(id);
                    return;
                };
                proposal.state = ProposalState::Queued;
                proposal.eta = Some(e.eta);
            }
            E::ProposalExecuted(e) => match self.proposals.get_mut(&e.proposal) {
                Some(proposal) => proposal.state = ProposalState::Executed,
                None => {
                    self.orphaned.insert(id);
                }
            },
            E::ProposalCancelled(e) => match self.proposals.get_mut(&e.proposal) {
                Some(proposal) => proposal.state = ProposalState::Cancelled,
                None => {
                    self.orphaned.insert(id);
                }
            },
            E::RealmInitialized(_) => {}

            // ------------------------------------------------------ treasury
            E::TreasuryInitialized(e) => {
                let treasury = self.treasuries.entry(e.treasury).or_default();
                treasury.governance_executor = Some(e.governance_executor);
                treasury.epoch_spend_cap = e.epoch_spend_cap;
            }
            E::Deposited(e) => {
                self.treasury_mut(e.treasury, &id).total_deposited = e.total_deposited;
            }
            E::Spent(e) => {
                self.treasury_mut(e.treasury, &id).total_spent = e.total_spent;
            }
            E::StreamCreated(e) => {
                self.treasury_mut(e.treasury, &id).open_streams.insert(
                    e.stream,
                    StreamStats {
                        beneficiary: e.beneficiary,
                        stream_id: e.stream_id,
                        total_amount: e.total_amount,
                        claimed: 0,
                        revoked: false,
                    },
                );
            }
            E::StreamClaimed(e) => {
                let orphan = id.clone();
                let treasury = self.treasury_mut(e.treasury, &id);
                treasury.total_stream_claims =
                    treasury.total_stream_claims.saturating_add(e.amount);
                match treasury.open_streams.get_mut(&e.stream) {
                    Some(stream) => stream.claimed = e.total_claimed,
                    None => {
                        self.orphaned.insert(orphan);
                    }
                }
            }
            E::StreamRevoked(e) => {
                let orphan = id.clone();
                let treasury = self.treasury_mut(e.treasury, &id);
                match treasury.open_streams.get_mut(&e.stream) {
                    Some(stream) => {
                        stream.revoked = true;
                        // Accrual stops here, so what was vested at this instant
                        // is all the stream can ever pay.
                        stream.total_amount = stream.claimed.saturating_add(e.vested_retained);
                    }
                    None => {
                        self.orphaned.insert(orphan);
                    }
                }
            }
            E::SpendCapChanged(e) => {
                self.treasury_mut(e.treasury, &id).epoch_spend_cap = e.new_cap;
            }
            E::GovernanceExecutorChanged(e) => {
                self.treasury_mut(e.treasury, &id).governance_executor = Some(e.new_executor);
            }

            // -------------------------------------------------- token-manager
            // Issuance is tracked by the token-manager's own accounts and adds
            // nothing to the analytics these views serve. Listed explicitly so
            // that adding an event forces a decision here rather than silently
            // falling into a catch-all.
            E::TokenInitialized(_)
            | E::MinterRegistered(_)
            | E::MinterUpdated(_)
            | E::MinterRevoked(_)
            | E::TokensMinted(_)
            | E::TokensBurned(_)
            | E::AdminTransferProposed(_)
            | E::AdminTransferCancelled(_)
            | E::AdminTransferAccepted(_)
            | E::PauseToggled(_) => {}
        }
    }
}

// --------------------------------------------------------------- read views

impl Analytics {
    /// Total value locked in a pool, in the stake mint's base units.
    pub fn tvl(&self, pool: &Pubkey) -> u64 {
        self.pools.get(pool).map_or(0, |p| p.total_staked)
    }

    /// Emissions per year against total stake, in basis points.
    ///
    /// `None` when nothing is staked — a rate over an empty pool is not an
    /// infinite APR, it is an undefined one, and returning a number here is how
    /// dashboards end up showing `∞%`.
    pub fn apr_bps(&self, pool: &Pubkey) -> Option<u64> {
        let pool = self.pools.get(pool)?;
        if pool.total_staked == 0 {
            return None;
        }
        const SECONDS_PER_YEAR: u128 = 365 * 24 * 60 * 60;
        let annual = (pool.reward_rate as u128).checked_mul(SECONDS_PER_YEAR)?;
        u64::try_from(annual.checked_mul(10_000)? / pool.total_staked as u128).ok()
    }

    /// Live positions in a pool, largest first.
    ///
    /// Fully withdrawn positions are excluded: the account still exists on chain
    /// (nothing closes it — see the roadmap's technical debt), but a zero-weight
    /// position is not a staker.
    pub fn staker_distribution(&self, pool: &Pubkey) -> Vec<(Pubkey, u64)> {
        let mut by_owner: BTreeMap<Pubkey, u64> = BTreeMap::new();
        for position in self.positions.values() {
            if position.pool == *pool && position.amount > 0 {
                *by_owner.entry(position.owner).or_default() += position.amount;
            }
        }
        let mut ranked: Vec<_> = by_owner.into_iter().collect();
        // Largest first, then by address so equal stakes have a stable order —
        // a leaderboard that reshuffles between identical queries looks broken.
        ranked.sort_by_key(|(owner, amount)| (Reverse(*amount), *owner));
        ranked
    }

    /// Proposals in a realm, newest first.
    pub fn proposal_history(&self, realm: &Pubkey) -> Vec<&ProposalStats> {
        let mut history: Vec<_> = self
            .proposals
            .values()
            .filter(|p| p.realm == *realm)
            .collect();
        history.sort_by_key(|p| Reverse(p.id));
        history
    }

    /// What the treasury still holds according to the event stream: deposits
    /// less everything that has left, by spend or by vesting claim.
    pub fn treasury_balance(&self, treasury: &Pubkey) -> u64 {
        self.treasuries.get(treasury).map_or(0, |t| {
            t.total_deposited
                .saturating_sub(t.total_spent)
                .saturating_sub(t.total_stream_claims)
        })
    }

    /// Tokens promised to vesting streams and not yet claimed.
    pub fn committed_to_streams(&self, treasury: &Pubkey) -> u64 {
        self.treasuries.get(treasury).map_or(0, |t| {
            t.open_streams
                .values()
                .map(|s| s.total_amount.saturating_sub(s.claimed))
                .sum()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Program;
    use crate::logs::EmittedEvent;

    fn emitted(event: HelixEvent, log_index: usize) -> EmittedEvent {
        EmittedEvent {
            program: event.program(),
            event,
            log_index,
            depth: 1,
        }
    }

    fn staked(pool: Pubkey, position: Pubkey, owner: Pubkey, amount: u64) -> HelixEvent {
        HelixEvent::Staked(helix_staking::events::Staked {
            pool,
            position,
            owner,
            position_id: 0,
            amount_sent: amount,
            amount_credited: amount,
            weighted_amount: LockTier::Bronze.apply_weight(amount).unwrap(),
            tier: LockTier::Bronze,
            lock_end: 100,
            timestamp: 1,
        })
    }

    #[test]
    fn a_redelivered_transaction_changes_nothing() {
        let (pool, position, owner) = (
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
        );
        let events = vec![emitted(staked(pool, position, owner, 1_000), 3)];

        let mut analytics = Analytics::new();
        assert_eq!(analytics.apply_transaction("sig", &events), 1);
        let after_first = analytics.pools[&pool].clone();

        // Confirmed logs can be delivered twice, and a backfill overlapping a
        // live stream replays whole ranges. Both must be free.
        assert_eq!(analytics.apply_transaction("sig", &events), 0);
        assert_eq!(analytics.pools[&pool], after_first);
        assert_eq!(analytics.applied_count(), 1);
    }

    /// Two identical events in one transaction are two events.
    ///
    /// This is why the key carries the log index. Deduplicating on the event's
    /// content, or on the signature alone, silently loses the second one.
    #[test]
    fn identical_events_in_one_transaction_are_not_deduplicated() {
        let (pool, owner) = (Pubkey::new_unique(), Pubkey::new_unique());
        let events = vec![
            emitted(staked(pool, Pubkey::new_unique(), owner, 1_000), 3),
            emitted(staked(pool, Pubkey::new_unique(), owner, 1_000), 7),
        ];

        let mut analytics = Analytics::new();
        assert_eq!(analytics.apply_transaction("sig", &events), 2);
        assert_eq!(analytics.pools[&pool].total_staked, 2_000);
        assert_eq!(analytics.pools[&pool].position_count, 2);
    }

    #[test]
    fn an_event_for_an_unknown_entity_is_recorded_as_orphaned() {
        let mut analytics = Analytics::new();
        let claimed = HelixEvent::RewardsClaimed(helix_staking::events::RewardsClaimed {
            pool: Pubkey::new_unique(),
            position: Pubkey::new_unique(),
            owner: Pubkey::new_unique(),
            amount: 5,
            timestamp: 1,
        });
        analytics.apply_transaction("sig", &[emitted(claimed, 0)]);

        // Not silently dropped: a backfill starting mid-history expects these,
        // a live stream seeing one has lost something.
        assert_eq!(analytics.orphaned.len(), 1);
    }

    #[test]
    fn apr_is_undefined_rather_than_infinite_on_an_empty_pool() {
        let pool = Pubkey::new_unique();
        let mut analytics = Analytics::new();
        analytics.apply_transaction(
            "sig",
            &[emitted(
                HelixEvent::RewardRateChanged(helix_staking::events::RewardRateChanged {
                    pool,
                    old_rate: 0,
                    new_rate: 1_000,
                    reward_period_end: 0,
                    timestamp: 1,
                }),
                0,
            )],
        );
        assert_eq!(analytics.apr_bps(&pool), None);
    }

    #[test]
    fn staker_distribution_excludes_fully_withdrawn_positions() {
        let (pool, owner) = (Pubkey::new_unique(), Pubkey::new_unique());
        let position = Pubkey::new_unique();
        let mut analytics = Analytics::new();
        analytics.apply_transaction("a", &[emitted(staked(pool, position, owner, 1_000), 0)]);
        assert_eq!(analytics.staker_distribution(&pool), vec![(owner, 1_000)]);

        analytics.apply_transaction(
            "b",
            &[emitted(
                HelixEvent::Unstaked(helix_staking::events::Unstaked {
                    pool,
                    position,
                    owner,
                    amount: 1_000,
                    remaining: 0,
                    weighted_amount: 0,
                    timestamp: 2,
                }),
                0,
            )],
        );
        assert!(analytics.staker_distribution(&pool).is_empty());
        assert_eq!(analytics.pools[&pool].total_staked, 0);
        assert_eq!(analytics.pools[&pool].total_weighted, 0);
    }

    #[test]
    fn program_attribution_survives_the_round_trip() {
        let event = staked(
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            1,
        );
        assert_eq!(event.program(), Program::Staking);
    }
}
