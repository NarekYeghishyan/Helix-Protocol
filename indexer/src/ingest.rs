//! Driving a [`LogSource`] into a projection, safely across reorgs.
//!
//! # The problem
//!
//! Confirmed is not final. A slot the cluster has already served can be rolled
//! back and replaced, so an indexer that folds every transaction it sees straight
//! into one projection has no way to *un*-fold the ones that turn out never to
//! have happened. It will be wrong, and — worse — it will be wrong quietly,
//! because nothing about the arithmetic looks unusual afterwards.
//!
//! # The shape of the fix
//!
//! Two projections and a buffer:
//!
//! ```text
//!   finalized  ── state through the cluster's finalized slot; never rewound
//!   pending    ── transactions above it, in order, kept so they can be replayed
//!   head       ── finalized + pending, which is what queries read
//! ```
//!
//! Every poll re-reads the whole unfinalised range rather than asking for a diff.
//! That is the point: a rollback shows up as the source disagreeing with the
//! buffer, which is something that can be detected, rather than as a transaction
//! simply never being mentioned again, which is not.
//!
//! On disagreement, `head` is rebuilt from `finalized` and the source's current
//! view is replayed over it. Rebuilding rather than reversing is deliberate —
//! inverting an arbitrary fold needs every projection field to have an inverse,
//! and `saturating_sub` does not.
//!
//! # What this cannot do
//!
//! A source that silently omits a transaction — an RPC provider dropping a log,
//! not a rollback — is indistinguishable from that transaction never existing.
//! Nothing here detects it. The defence is the `orphaned` set in
//! [`crate::projection`]: a later event referring to an entity that was never
//! created is the symptom, and it is reported.

use crate::logs::{parse, Anomaly};
use crate::projection::Analytics;
use crate::source::{Cursor, LogSource, TransactionLogs};

/// A transaction that has been applied but is not yet final.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Pending {
    signature: String,
    slot: u64,
}

/// What one [`Ingestor::poll`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PollOutcome {
    /// Transactions applied to `head` that had not been applied before.
    pub applied: usize,
    /// How many buffered transactions a rollback invalidated, if any.
    pub reverted: Option<usize>,
    /// Transactions promoted from pending to finalised this poll.
    pub finalized: usize,
    /// Log-level problems seen, e.g. truncation. Never silently dropped.
    pub anomalies: Vec<Anomaly>,
}

impl PollOutcome {
    pub fn was_reorg(&self) -> bool {
        self.reverted.is_some()
    }
}

#[derive(Debug)]
pub enum IngestError<E> {
    Source(E),
    /// The source contradicted itself below the finality watermark: a
    /// transaction this ingestor had already finalised is gone or has moved.
    ///
    /// Not a reorg — finalised history does not change. Either the source is
    /// lying, it is a different cluster, or the stored cursor belongs to another
    /// ledger entirely. Ingestion stops rather than papering over it, because
    /// every number downstream is now suspect.
    FinalizedHistoryChanged {
        slot: u64,
        signature: String,
    },
}

pub struct Ingestor {
    /// State through `cursor.slot`. Only ever moves forward.
    finalized: Analytics,
    /// `finalized` plus everything in `pending`. What queries should read.
    head: Analytics,
    pending: Vec<Pending>,
    cursor: Cursor,
    /// Every transaction seen since the last finalisation, kept so a replay after
    /// a rollback re-applies them in order.
    replay: Vec<TransactionLogs>,
}

impl Default for Ingestor {
    fn default() -> Self {
        Self::new()
    }
}

impl Ingestor {
    pub fn new() -> Self {
        Self {
            finalized: Analytics::new(),
            head: Analytics::new(),
            pending: Vec::new(),
            cursor: Cursor::default(),
            replay: Vec::new(),
        }
    }

    /// Resumes from a persisted cursor.
    ///
    /// The projection starts empty, so this is a resume for *ingestion* and not
    /// for state — a real deployment pairs it with a loaded projection. What it
    /// establishes is that the source is asked to continue rather than to replay
    /// the chain from genesis, and that anything below the cursor is treated as
    /// settled.
    pub fn resume_at(cursor: Cursor) -> Self {
        Self {
            cursor,
            ..Self::new()
        }
    }

    /// What queries read: finalised state plus everything not yet final.
    pub fn head(&self) -> &Analytics {
        &self.head
    }

    /// State the cluster will not take back.
    pub fn finalized(&self) -> &Analytics {
        &self.finalized
    }

    pub fn cursor(&self) -> &Cursor {
        &self.cursor
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Reads one batch from `source` and folds it in.
    pub fn poll<S: LogSource>(
        &mut self,
        source: &mut S,
        limit: usize,
    ) -> Result<PollOutcome, IngestError<S::Error>> {
        let finalized_slot = source.finalized_slot().map_err(IngestError::Source)?;
        let batch = source
            .fetch(&self.cursor, limit)
            .map_err(IngestError::Source)?;

        // Checked before anything is applied, so a source serving a ledger this
        // cursor does not belong to leaves the projection untouched rather than
        // half-written. A transaction *at* `cursor.slot` is fine — the cursor
        // resumes mid-slot by signature, so re-seeing its siblings is ordinary,
        // and idempotency absorbs them.
        if let Some(contradiction) = batch.iter().find(|tx| tx.slot < self.cursor.slot) {
            return Err(IngestError::FinalizedHistoryChanged {
                slot: contradiction.slot,
                signature: contradiction.signature.clone(),
            });
        }

        let mut outcome = PollOutcome::default();

        // How much of what we already applied the source still agrees with.
        let agreed = self
            .replay
            .iter()
            .zip(&batch)
            .take_while(|(had, now)| had.signature == now.signature && had.slot == now.slot)
            .count();

        if agreed < self.replay.len() {
            // The source no longer reports something we applied. Everything from
            // the divergence onward has to go.
            outcome.reverted = Some(self.replay.len() - agreed);
            self.rebuild_head_from_finalized();
            self.replay.truncate(agreed);
            for tx in &self.replay.clone() {
                self.apply(tx, &mut outcome);
            }
        }

        for tx in batch.iter().skip(agreed) {
            self.apply(tx, &mut outcome);
            self.replay.push(tx.clone());
        }

        self.advance_finality(finalized_slot, &mut outcome);
        Ok(outcome)
    }

    /// Folds one transaction into `head` and records it as pending.
    fn apply(&mut self, tx: &TransactionLogs, outcome: &mut PollOutcome) {
        let parsed = parse(&tx.logs);
        outcome.anomalies.extend(parsed.anomalies.iter().cloned());

        let new = self.head.apply_transaction(&tx.signature, &parsed.events);
        if new > 0 {
            outcome.applied += 1;
        }
        self.pending.push(Pending {
            signature: tx.signature.clone(),
            slot: tx.slot,
        });
    }

    fn rebuild_head_from_finalized(&mut self) {
        self.head = self.finalized.clone();
        self.pending.clear();
    }

    /// Promotes everything at or below `finalized_slot` into `finalized`.
    fn advance_finality(&mut self, finalized_slot: u64, outcome: &mut PollOutcome) {
        let settling = self
            .replay
            .iter()
            .take_while(|tx| tx.slot <= finalized_slot)
            .count();

        if settling == 0 {
            return;
        }

        for tx in self.replay.drain(..settling) {
            let parsed = parse(&tx.logs);
            self.finalized
                .apply_transaction(&tx.signature, &parsed.events);
            self.cursor = Cursor {
                slot: tx.slot,
                signature: Some(tx.signature),
            };
        }

        self.pending.drain(..settling.min(self.pending.len()));
        outcome.finalized = settling;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::ScriptedSource;
    use anchor_lang::prelude::Pubkey;
    use anchor_lang::{AnchorSerialize, Discriminator};
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine as _;
    use helix_staking::state::LockTier;

    /// A log containing one `Staked` event for `amount`.
    fn staked_log(pool: Pubkey, position: Pubkey, owner: Pubkey, amount: u64) -> Vec<String> {
        let event = helix_staking::events::Staked {
            pool,
            position,
            owner,
            position_id: 0,
            amount_sent: amount,
            amount_credited: amount,
            weighted_amount: amount,
            tier: LockTier::Flexible,
            lock_end: 0,
            timestamp: 1,
        };
        let mut bytes = helix_staking::events::Staked::DISCRIMINATOR.to_vec();
        event.serialize(&mut bytes).expect("serialize");

        vec![
            format!("Program {} invoke [1]", helix_staking::ID),
            format!("Program data: {}", BASE64.encode(bytes)),
            format!("Program {} success", helix_staking::ID),
        ]
    }

    struct Fixture {
        pool: Pubkey,
        source: ScriptedSource,
    }

    impl Fixture {
        /// A ledger of `n` stakes of 100 each, one per slot from 1.
        fn with_stakes(n: u64) -> Self {
            let pool = Pubkey::new_unique();
            let mut source = ScriptedSource::new();
            for i in 1..=n {
                source.push(
                    &format!("sig-{i}"),
                    i,
                    staked_log(pool, Pubkey::new_unique(), Pubkey::new_unique(), 100),
                );
            }
            Self { pool, source }
        }

        fn staked(&self, ingestor: &Ingestor) -> u64 {
            ingestor.head().tvl(&self.pool)
        }

        fn finalized_staked(&self, ingestor: &Ingestor) -> u64 {
            ingestor.finalized().tvl(&self.pool)
        }
    }

    #[test]
    fn a_linear_ledger_is_ingested_once() {
        let mut f = Fixture::with_stakes(3);
        f.source.finalize_through(3);

        let mut ingestor = Ingestor::new();
        let outcome = ingestor.poll(&mut f.source, 100).expect("poll");

        assert_eq!(outcome.applied, 3);
        assert_eq!(outcome.finalized, 3);
        assert!(!outcome.was_reorg());
        assert_eq!(f.staked(&ingestor), 300);
        assert_eq!(f.finalized_staked(&ingestor), 300);
        assert_eq!(ingestor.pending_count(), 0);
    }

    /// Polling again with nothing new must change nothing.
    #[test]
    fn polling_an_unchanged_ledger_is_a_no_op() {
        let mut f = Fixture::with_stakes(3);
        f.source.finalize_through(3);

        let mut ingestor = Ingestor::new();
        ingestor.poll(&mut f.source, 100).expect("first");
        let after_first = f.staked(&ingestor);

        let outcome = ingestor.poll(&mut f.source, 100).expect("second");
        assert_eq!(outcome.applied, 0);
        assert_eq!(f.staked(&ingestor), after_first);
    }

    /// Everything above the finality watermark stays revocable.
    #[test]
    fn unfinalized_transactions_are_held_as_pending() {
        let mut f = Fixture::with_stakes(3);
        f.source.finalize_through(1);

        let mut ingestor = Ingestor::new();
        let outcome = ingestor.poll(&mut f.source, 100).expect("poll");

        assert_eq!(outcome.applied, 3);
        assert_eq!(outcome.finalized, 1, "only slot 1 is final");
        assert_eq!(f.staked(&ingestor), 300, "head sees all three");
        assert_eq!(
            f.finalized_staked(&ingestor),
            100,
            "finalized state stops at the watermark"
        );
        assert_eq!(ingestor.pending_count(), 2);
    }

    /// The case the whole design exists for.
    #[test]
    fn a_rollback_above_the_watermark_is_reverted_and_replaced() {
        let mut f = Fixture::with_stakes(3);
        f.source.finalize_through(1);

        let mut ingestor = Ingestor::new();
        ingestor.poll(&mut f.source, 100).expect("first");
        assert_eq!(f.staked(&ingestor), 300);

        // Slots 2 and 3 are rolled back and replaced by one different
        // transaction — the ordinary shape of a fork being resolved.
        f.source.roll_back_to(1);
        f.source.push(
            "sig-replacement",
            2,
            staked_log(f.pool, Pubkey::new_unique(), Pubkey::new_unique(), 100),
        );

        let outcome = ingestor.poll(&mut f.source, 100).expect("second");

        assert!(outcome.was_reorg(), "the rollback went unnoticed");
        assert_eq!(outcome.reverted, Some(2));
        assert_eq!(
            f.staked(&ingestor),
            200,
            "two rolled-back stakes were not removed, or the replacement was lost"
        );
        assert_eq!(f.finalized_staked(&ingestor), 100, "finalized is untouched");
    }

    /// A rollback that removes transactions without replacing them.
    #[test]
    fn a_rollback_to_nothing_leaves_only_finalized_state() {
        let mut f = Fixture::with_stakes(4);
        f.source.finalize_through(2);

        let mut ingestor = Ingestor::new();
        ingestor.poll(&mut f.source, 100).expect("first");
        assert_eq!(f.staked(&ingestor), 400);

        f.source.roll_back_to(2);
        let outcome = ingestor.poll(&mut f.source, 100).expect("second");

        assert_eq!(outcome.reverted, Some(2));
        assert_eq!(f.staked(&ingestor), 200);
        assert_eq!(f.finalized_staked(&ingestor), 200);
        assert_eq!(ingestor.pending_count(), 0);
    }

    /// Finalised history changing is not a reorg, and must not be treated as one.
    #[test]
    fn a_change_below_the_watermark_stops_ingestion() {
        let mut f = Fixture::with_stakes(3);
        f.source.finalize_through(3);

        let mut ingestor = Ingestor::new();
        ingestor.poll(&mut f.source, 100).expect("first");
        assert_eq!(ingestor.cursor().slot, 3);

        // The source now serves a *different* ledger whose slot 2 was never seen.
        // Resuming from a cursor that belongs to another chain is the realistic
        // way this happens: a stored cursor pointed at the wrong cluster.
        f.source.ledger.clear();
        f.source.push(
            "sig-from-another-chain",
            2,
            staked_log(f.pool, Pubkey::new_unique(), Pubkey::new_unique(), 100),
        );

        let err = ingestor
            .poll(&mut f.source, 100)
            .expect_err("finalised history changed and ingestion continued");
        assert!(matches!(
            err,
            IngestError::FinalizedHistoryChanged { slot: 2, .. }
        ));
    }

    /// Ingestion in small batches reaches the same state as one large one.
    #[test]
    fn a_paged_backfill_matches_a_single_pass() {
        let one_pass = {
            let mut f = Fixture::with_stakes(6);
            f.source.finalize_through(6);
            let mut ingestor = Ingestor::new();
            ingestor.poll(&mut f.source, 100).expect("poll");
            f.staked(&ingestor)
        };

        let mut f = Fixture::with_stakes(6);
        f.source.finalize_through(6);
        let mut ingestor = Ingestor::new();
        for _ in 0..6 {
            ingestor.poll(&mut f.source, 2).expect("paged poll");
        }

        assert_eq!(f.staked(&ingestor), one_pass);
        assert_eq!(f.staked(&ingestor), 600);
    }

    /// A crashed indexer resumes from its cursor instead of from genesis.
    #[test]
    fn resuming_from_a_cursor_does_not_re_read_finalised_history() {
        let mut f = Fixture::with_stakes(4);
        f.source.finalize_through(4);

        // Pretend slots 1-2 were ingested and persisted by a previous process.
        let mut ingestor = Ingestor::resume_at(Cursor {
            slot: 2,
            signature: Some("sig-2".into()),
        });
        let outcome = ingestor.poll(&mut f.source, 100).expect("poll");

        assert_eq!(
            outcome.applied, 2,
            "resumed run re-ingested history it was told to skip"
        );
        assert_eq!(
            f.staked(&ingestor),
            200,
            "only slots 3 and 4 should have been applied"
        );
        assert_eq!(ingestor.cursor().slot, 4);
    }

    /// A truncated log is surfaced by the ingestor, not swallowed on the way up.
    #[test]
    fn log_anomalies_reach_the_poll_outcome() {
        let pool = Pubkey::new_unique();
        let mut source = ScriptedSource::new();
        let mut logs = staked_log(pool, Pubkey::new_unique(), Pubkey::new_unique(), 100);
        logs.insert(2, "Log truncated".into());
        source.push("sig-truncated", 1, logs);
        source.finalize_through(1);

        let mut ingestor = Ingestor::new();
        let outcome = ingestor.poll(&mut source, 100).expect("poll");

        assert!(
            outcome
                .anomalies
                .iter()
                .any(|a| matches!(a, Anomaly::Truncated { .. })),
            "truncation did not reach the caller: {:?}",
            outcome.anomalies
        );
    }
}
