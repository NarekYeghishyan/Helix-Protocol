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

use crate::logs::{parse, Anomaly, EmittedEvent};
use crate::projection::Analytics;
use crate::source::{Cursor, DescendingSource, Descent, LogSource, TransactionLogs};

/// A transaction that has been applied but is not yet final.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Pending {
    signature: String,
    slot: u64,
}

/// A transaction the cluster will not take back, with what it emitted.
///
/// Carried out of a poll rather than merely counted, because it is exactly what a
/// durable store may write. Anything still pending may be rolled back, and a
/// database row that a fork can revoke is a number that was never true — so the
/// events cross this boundary at the moment they finalise and not before.
#[derive(Clone, Debug, PartialEq)]
pub struct SettledTransaction {
    pub signature: String,
    pub slot: u64,
    pub events: Vec<EmittedEvent>,
    /// What was wrong with this transaction's log, if anything.
    ///
    /// Also reported live in [`PollOutcome::anomalies`], and the duplication is
    /// deliberate: an operator needs to know about a truncated log the moment it
    /// is seen, and a store must only record one for a transaction that actually
    /// happened. Those are different moments, and a rollback between them is the
    /// case that makes them different.
    pub anomalies: Vec<Anomaly>,
}

/// A log-level problem, and the transaction it was seen in.
///
/// [`Anomaly`] on its own names a line number in a log the reader has no way to
/// find again — `Truncated { log_index: 12 }` is a statistic, not something
/// anyone can act on. The signature is what makes it a report, and it is what the
/// `ingestion_anomalies` table has had in its primary key since it was written,
/// during the whole period the code could not supply it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportedAnomaly {
    pub signature: String,
    pub anomaly: Anomaly,
}

/// What one [`Ingestor::poll`] did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PollOutcome {
    /// Transactions applied to `head` that had not been applied before.
    pub applied: usize,
    /// How many buffered transactions a rollback invalidated, if any.
    pub reverted: Option<usize>,
    /// Transactions promoted from pending to finalised this poll, in ledger
    /// order. The cursor after the poll points at the last of them.
    pub settled: Vec<SettledTransaction>,
    /// Log-level problems seen, e.g. truncation. Never silently dropped.
    pub anomalies: Vec<ReportedAnomaly>,
}

impl PollOutcome {
    pub fn was_reorg(&self) -> bool {
        self.reverted.is_some()
    }

    /// How many transactions this poll finalised.
    pub fn finalized(&self) -> usize {
        self.settled.len()
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

/// What one [`Backfill::step`] read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BackfillBatch {
    /// Settled transactions from this page, oldest first.
    ///
    /// The same type a poll emits, and for the same reason: this is exactly what
    /// a durable store may write. What a store must **not** do with them is fold
    /// them into a live projection — see [`Backfill`].
    pub transactions: Vec<SettledTransaction>,
    /// Transactions this page skipped for being above the finality watermark.
    ///
    /// Not an error and not silent. A descent that starts at the tip necessarily
    /// begins inside the unfinalised range, and those transactions belong to the
    /// live stream; counting them makes the hand-off visible rather than
    /// assumed.
    pub skipped_unfinalized: usize,
    /// True once the descent has reached the beginning of history.
    pub complete: bool,
}

/// A descending traversal, from the tip toward genesis.
///
/// # Why this is not just `Ingestor` run backwards
///
/// The projection is built to be replay-safe by *assignment*: events carry
/// running totals, and folding one twice sets the same field to the same value.
/// That property is what makes redelivery harmless, and it depends entirely on
/// events arriving in ledger order. Fold an older `RewardRateChanged` after a
/// newer one and the newer rate is simply overwritten by the older — no error, no
/// anomaly, a plausible wrong number.
///
/// So a backfill cannot fold into the live projection as it goes, and the fix is
/// not to make the fold order-independent. It is to send the backfill somewhere
/// order *does not matter*: the `events` table, whose key is
/// `(signature, log_index)` and whose stated purpose is that everything else is
/// derivable from it. The projection is then rebuilt from those rows in slot
/// order — see [`Analytics::replay`] — rather than being nudged backwards.
///
/// That is why [`BackfillBatch`] carries [`SettledTransaction`]s and no
/// projection at all. A `Backfill` that owned an `Analytics` would invite exactly
/// the fold this type exists to prevent.
///
/// # Where it meets the live stream
///
/// The live cursor never exceeds the finality watermark, and this refuses to emit
/// anything above it. So the live stream owns `[cursor, tip]` and the descent owns
/// `[genesis, descent)` — and they overlap in the middle rather than meeting
/// exactly, which is fine because both write the same idempotent rows. Requiring
/// them to meet exactly would need coordination neither has.
#[derive(Clone, Debug, Default)]
pub struct Backfill {
    descent: Descent,
}

impl Backfill {
    /// Starts at the tip.
    pub fn new() -> Self {
        Self::default()
    }

    /// Resumes from a persisted descent — the `backfill` row of `cursors`.
    pub fn resume_at(descent: Descent) -> Self {
        Self { descent }
    }

    pub fn descent(&self) -> &Descent {
        &self.descent
    }

    /// Whether there is anything left to read.
    pub fn is_complete(&self) -> bool {
        self.descent.complete
    }

    /// Reads one page older than the current position.
    ///
    /// Returns an empty, `complete` batch once genesis is reached, and keeps
    /// returning one thereafter — a caller looping until `complete` terminates,
    /// and a caller that keeps stepping wastes a request rather than restarting
    /// from the tip.
    pub fn step<S: DescendingSource>(
        &mut self,
        source: &mut S,
        limit: usize,
    ) -> Result<BackfillBatch, IngestError<S::Error>> {
        if self.descent.complete {
            return Ok(BackfillBatch {
                complete: true,
                ..Default::default()
            });
        }

        let finalized_slot = source.finalized_slot().map_err(IngestError::Source)?;
        let page = source
            .fetch_before(&self.descent, limit)
            .map_err(IngestError::Source)?;

        // Nothing older exists at all. Genesis, and the only way this terminates.
        //
        // Note that this is `covered`, not `transactions` — a page of nothing but
        // failed transactions covers a range and yields no transactions, and
        // reading that as the end of history would stop the descent early while
        // reporting the rest as complete. See `DescentPage`.
        let Some(covered) = page.covered else {
            self.descent.complete = true;
            return Ok(BackfillBatch {
                complete: true,
                ..Default::default()
            });
        };

        // A source that hands back something at a higher slot than the bound has
        // either ignored it or is serving a different ledger. Either way the next
        // step would ask the same question and get the same answer forever, so it
        // is refused rather than absorbed — the same stance `poll` takes on a
        // contradiction below its own cursor.
        //
        // Checked against the *newest* entry of the page. The oldest cannot catch
        // it: a source ignoring the bound serves history from the tip, whose
        // oldest entry is genesis and therefore below any bound at all.
        if let Some(bound) = self.descent.slot {
            if covered.newest.slot > bound {
                return Err(IngestError::FinalizedHistoryChanged {
                    slot: covered.newest.slot,
                    signature: covered.newest.signature,
                });
            }
        }

        // Advance past the whole page, including anything filtered out of it.
        self.descent.slot = Some(covered.oldest.slot);
        self.descent.signature = Some(covered.oldest.signature);

        let mut batch = BackfillBatch::default();
        for tx in page.transactions {
            // Above the watermark this is not history yet, and a row a fork can
            // revoke is a number that was never true. The live stream has it.
            if tx.slot > finalized_slot {
                batch.skipped_unfinalized += 1;
                continue;
            }

            let parsed = parse(&tx.logs);
            batch.transactions.push(SettledTransaction {
                signature: tx.signature,
                slot: tx.slot,
                events: parsed.events,
                anomalies: parsed.anomalies,
            });
        }

        Ok(batch)
    }
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

    /// Resumes ingestion from a persisted cursor, with an empty projection.
    ///
    /// This is a resume for *ingestion* and not for state: the source is asked to
    /// continue rather than to replay the chain from genesis, and anything below
    /// the cursor is treated as settled — but nothing below it is *known*. Use
    /// [`Self::restore`] when the projection has been loaded too.
    pub fn resume_at(cursor: Cursor) -> Self {
        Self {
            cursor,
            ..Self::new()
        }
    }

    /// Resumes from a persisted cursor **and** the state that cursor describes.
    ///
    /// The two must come from the same read, or the indexer resumes at a slot
    /// whose events are not all in the projection and silently never fetches
    /// them again. That is why [`crate::store::Store::load`] returns both from
    /// one snapshot rather than offering them separately.
    ///
    /// `head` starts as a copy of `finalized`: everything above the cursor is
    /// unfinalised, so it is re-read from the source rather than persisted. That
    /// is a few seconds of chain, and it is the reason no stored row is ever
    /// subject to a rollback.
    pub fn restore(cursor: Cursor, finalized: Analytics) -> Self {
        Self {
            head: finalized.clone(),
            finalized,
            pending: Vec::new(),
            cursor,
            replay: Vec::new(),
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
        outcome.anomalies.extend(
            parsed
                .anomalies
                .iter()
                .cloned()
                .map(|anomaly| ReportedAnomaly {
                    signature: tx.signature.clone(),
                    anomaly,
                }),
        );

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
                signature: Some(tx.signature.clone()),
            };
            outcome.settled.push(SettledTransaction {
                signature: tx.signature,
                slot: tx.slot,
                events: parsed.events,
                anomalies: parsed.anomalies,
            });
        }

        self.pending.drain(..settling.min(self.pending.len()));
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
        assert_eq!(outcome.finalized(), 3);
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
        assert_eq!(outcome.finalized(), 1, "only slot 1 is final");
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
                .any(|a| matches!(a.anomaly, Anomaly::Truncated { .. })),
            "truncation did not reach the caller: {:?}",
            outcome.anomalies
        );

        // Without the signature there is nothing to re-fetch, which is the only
        // thing anyone can actually do about a truncated log.
        assert_eq!(outcome.anomalies[0].signature, "sig-truncated");

        // And it must reach the settled record too, or a store writing only what
        // finalised would record the events while losing the fact that they are
        // known to be incomplete.
        assert_eq!(outcome.settled.len(), 1);
        assert!(!outcome.settled[0].anomalies.is_empty());
    }

    // -------------------------------------------------------------- backfill
    //
    // A descent is the traversal the live poll cannot do, and its failure modes
    // are different ones. What matters here is not that it reads transactions —
    // that is `ScriptedSource` working — but that it terminates for the right
    // reason, that paging it changes nothing, and that what comes out the far
    // end reconstructs the projection a forward pass builds.

    /// Runs a descent to completion and returns everything it yielded, oldest
    /// first.
    fn descend_all(source: &mut ScriptedSource, page: usize) -> Vec<SettledTransaction> {
        let mut backfill = Backfill::new();
        let mut collected: Vec<SettledTransaction> = Vec::new();

        // Bounded, so a descent that fails to descend fails the test rather than
        // hanging it.
        for _ in 0..1_000 {
            let batch = backfill.step(source, page).expect("step");
            // Each page is older than the last, so prepending is what puts the
            // whole descent back into ledger order.
            let mut older = batch.transactions;
            older.extend(collected);
            collected = older;

            if batch.complete {
                return collected;
            }
        }
        panic!("descent did not terminate");
    }

    #[test]
    fn a_descent_reaches_genesis_and_stops() {
        let mut f = Fixture::with_stakes(5);
        f.source.finalize_through(5);

        let mut backfill = Backfill::new();
        assert!(!backfill.is_complete());

        let mut collected: Vec<SettledTransaction> = Vec::new();
        loop {
            let batch = backfill.step(&mut f.source, 2).expect("step");
            let mut older = batch.transactions;
            older.extend(collected);
            collected = older;
            if batch.complete {
                break;
            }
        }

        assert_eq!(collected.len(), 5);
        assert!(backfill.is_complete());
        assert_eq!(backfill.descent().slot, Some(1));

        // Stepping a finished descent is a no-op rather than a restart from the
        // tip, which is the whole reason `complete` is stored rather than
        // inferred from the cursor.
        let fetches = f.source.fetches;
        let after = backfill.step(&mut f.source, 2).expect("step");
        assert!(after.complete);
        assert!(after.transactions.is_empty());
        assert_eq!(f.source.fetches, fetches, "a finished descent asked again");
    }

    #[test]
    fn paging_a_descent_changes_nothing() {
        // The mirror of `a_paged_backfill_matches_a_single_pass`, for the other
        // direction. A page boundary is where an off-by-one silently drops or
        // repeats a transaction, and the descent's bound is computed *from* the
        // page rather than handed to it.
        let single = {
            let mut f = Fixture::with_stakes(9);
            f.source.finalize_through(9);
            descend_all(&mut f.source, 100)
        };
        assert_eq!(single.len(), 9);

        for page in [1, 2, 3, 4, 8, 9] {
            let mut f = Fixture::with_stakes(9);
            f.source.finalize_through(9);
            let paged = descend_all(&mut f.source, page);

            assert_eq!(
                paged.iter().map(|t| &t.signature).collect::<Vec<_>>(),
                single.iter().map(|t| &t.signature).collect::<Vec<_>>(),
                "page size {page} produced a different history",
            );
        }
    }

    #[test]
    fn a_descent_resumes_where_it_stopped() {
        let mut f = Fixture::with_stakes(6);
        f.source.finalize_through(6);

        let mut first = Backfill::new();
        let batch = first.step(&mut f.source, 2).expect("step");
        assert_eq!(batch.transactions.len(), 2);

        // What a store would persist under the `backfill` cursor, and nothing
        // else.
        let persisted = first.descent().clone();
        assert_eq!(persisted.slot, Some(5));
        assert!(!persisted.complete);

        let mut resumed = Backfill::resume_at(persisted);
        let mut rest: Vec<SettledTransaction> = Vec::new();
        loop {
            let batch = resumed.step(&mut f.source, 2).expect("step");
            let mut older = batch.transactions;
            older.extend(rest);
            rest = older;
            if batch.complete {
                break;
            }
        }

        // Four left, and neither of the two already read.
        assert_eq!(
            rest.iter().map(|t| t.slot).collect::<Vec<_>>(),
            vec![1, 2, 3, 4]
        );
    }

    #[test]
    fn a_descent_will_not_claim_the_unfinalised_tail() {
        // The hand-off. A descent starts at the tip, which is inside the range
        // the live stream owns, and a row a fork can still revoke is a number
        // that was never true. Skipped rather than refused, and counted rather
        // than silent.
        let mut f = Fixture::with_stakes(6);
        f.source.finalize_through(4);

        let mut backfill = Backfill::new();
        let first = backfill.step(&mut f.source, 3).expect("step");

        assert_eq!(
            first.skipped_unfinalized, 2,
            "slots 5 and 6 are not history"
        );
        assert_eq!(
            first
                .transactions
                .iter()
                .map(|t| t.slot)
                .collect::<Vec<_>>(),
            vec![4]
        );

        // And the descent moved past them regardless, so it does not sit on the
        // tail waiting for it to finalise. The live stream owns that range.
        assert_eq!(backfill.descent().slot, Some(4));
    }

    #[test]
    fn a_page_with_nothing_to_fold_is_not_genesis() {
        // The bug the shape of `DescentPage` exists to prevent. On a real cluster
        // a whole page can be transactions whose writes were rolled back —
        // ordinary for a program someone is spamming — and reading "nothing to
        // fold" as "no more history" stops the descent early while reporting the
        // rest complete.
        //
        // `ScriptedSource` has no notion of a failed transaction, so the
        // equivalent here is a page whose logs carry no events: same shape,
        // nothing to fold, and history below it.
        let pool = Pubkey::new_unique();
        let mut source = ScriptedSource::new();
        source.push(
            "sig-oldest",
            1,
            staked_log(pool, Pubkey::new_unique(), Pubkey::new_unique(), 100),
        );
        source.push("sig-noise-a", 2, vec!["Program log: nothing".into()]);
        source.push("sig-noise-b", 3, vec!["Program log: nothing".into()]);
        source.finalize_through(3);

        let all = descend_all(&mut source, 2);

        // The first page folds nothing, and the descent still reaches slot 1.
        assert_eq!(all.len(), 3);
        assert!(
            all.iter().any(|t| t.slot == 1 && !t.events.is_empty()),
            "the descent stopped before the only transaction carrying events",
        );
    }

    #[test]
    fn a_source_that_does_not_descend_is_refused() {
        // A bound the source ignores makes every step return the same page, and a
        // loop over `complete` never ends. Refused for the same reason a
        // contradiction below the live cursor is: the source is not serving the
        // ledger this cursor belongs to.
        let mut f = Fixture::with_stakes(4);
        f.source.finalize_through(4);

        let mut backfill = Backfill::resume_at(Descent {
            slot: Some(2),
            // Not in this ledger. `ScriptedSource` then serves from the tip,
            // which is what an RPC provider does with an unknown `before`.
            signature: Some("sig-from-another-cluster".into()),
            complete: false,
        });

        let err = backfill
            .step(&mut f.source, 10)
            .expect_err("a non-descending source was accepted");
        assert!(matches!(err, IngestError::FinalizedHistoryChanged { .. }));
    }

    #[test]
    fn a_backfilled_history_replays_into_the_projection_a_forward_pass_builds() {
        // The property the whole traversal exists for, and the reason `Backfill`
        // carries no `Analytics` of its own.
        let mut forward = Fixture::with_stakes(8);
        forward.source.finalize_through(8);
        let mut ingestor = Ingestor::new();
        ingestor.poll(&mut forward.source, 100).expect("poll");

        let mut backward = Fixture::with_stakes(8);
        backward.source.finalize_through(8);
        let descended = descend_all(&mut backward.source, 3);

        let rebuilt = Analytics::replay(
            descended
                .iter()
                .map(|t| (t.signature.as_str(), t.events.as_slice())),
        );

        assert_eq!(
            rebuilt.applied_count(),
            ingestor.finalized().applied_count()
        );
        assert_eq!(rebuilt.pools.len(), ingestor.finalized().pools.len());
        assert_eq!(
            rebuilt.tvl(&backward.pool),
            ingestor.finalized().tvl(&forward.pool),
        );
    }

    #[test]
    fn folding_a_descent_in_arrival_order_is_the_bug_replay_avoids() {
        // Why `BackfillBatch` carries transactions and not a projection.
        //
        // `RewardRateChanged` assigns a running value the event carries, which is
        // what makes redelivery harmless — and what makes *out-of-order* delivery
        // silently wrong. Applied newest-first, the older rate wins. No error, no
        // anomaly, a plausible number.
        let pool = Pubkey::new_unique();

        let rate_log = |rate: u64| {
            let event = helix_staking::events::RewardRateChanged {
                pool,
                old_rate: 0,
                new_rate: rate,
                reward_period_end: 0,
                timestamp: 1,
            };
            let mut bytes = helix_staking::events::RewardRateChanged::DISCRIMINATOR.to_vec();
            event.serialize(&mut bytes).expect("serialize");
            vec![
                format!("Program {} invoke [1]", helix_staking::ID),
                format!("Program data: {}", BASE64.encode(bytes)),
                format!("Program {} success", helix_staking::ID),
            ]
        };

        let mut source = ScriptedSource::new();
        source.push("sig-old-rate", 1, rate_log(100));
        source.push("sig-new-rate", 2, rate_log(900));
        source.finalize_through(2);

        let descended = descend_all(&mut source, 1);

        let ordered = Analytics::replay(
            descended
                .iter()
                .map(|t| (t.signature.as_str(), t.events.as_slice())),
        );
        assert_eq!(
            ordered.pools[&pool].reward_rate, 900,
            "ledger order should leave the newest rate standing"
        );

        // And the same events in arrival order — newest page first — do not.
        let arrival = Analytics::replay(
            descended
                .iter()
                .rev()
                .map(|t| (t.signature.as_str(), t.events.as_slice())),
        );
        assert_eq!(
            arrival.pools[&pool].reward_rate, 100,
            "if this ever reports 900, assignment stopped being order-dependent \
             and `Analytics::replay`'s precondition needs revisiting"
        );
    }
}
