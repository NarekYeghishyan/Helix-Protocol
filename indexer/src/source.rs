//! Where transaction logs come from, and how far the chain has committed to them.
//!
//! A trait rather than an RPC client, for the same reason the decoder is a pure
//! function: the interesting failures — rollbacks, gaps, redelivery, resuming
//! after a crash — are all expressible without a network, and none of them can be
//! *tested* with one. A scripted source can roll a slot back on demand; devnet
//! cannot be asked to.
//!
//! Two traversals live here, and they are deliberately separate traits.
//! [`LogSource`] is the live poll: it asks what is new, re-reads its own
//! unfinalised tail so a rollback shows up as a disagreement, and never
//! terminates. [`DescendingSource`] is the backfill: it asks what came before,
//! reads only settled history, and finishes. Everything that decides what to *do*
//! with what either returns lives in [`crate::ingest`].

/// One transaction's logs, as a source reports them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransactionLogs {
    pub signature: String,
    pub slot: u64,
    pub logs: Vec<String>,
}

/// How far ingestion has got.
///
/// Persisted between runs — it is the whole of what a resumable backfill needs to
/// remember, and the reason a crashed indexer restarts where it stopped rather
/// than at genesis.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    /// Everything at or below this slot has been ingested *and* finalised.
    pub slot: u64,
    /// The last signature applied at `slot`, so a slot containing several
    /// transactions can be resumed mid-way rather than re-read whole.
    pub signature: Option<String>,
}

/// How far a descending backfill has reached.
///
/// The inverse of [`Cursor`], and a separate type because it means the opposite
/// thing: everything at or **above** `slot` has been read. Reusing `Cursor` with
/// an inverted sense would have compiled everywhere and been wrong in exactly one
/// direction, which is the worst available outcome.
///
/// Persisted under the `backfill` key of the `cursors` table — the second cursor
/// the schema has always allowed. Two of them, because they move in opposite
/// directions and must not overwrite one another.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Descent {
    /// The slot of the oldest transaction read so far. `None` before the first
    /// page, which starts the descent at the tip.
    pub slot: Option<u64>,
    /// The oldest signature read so far — the exclusive upper bound of the next
    /// page, and what an RPC `before` parameter takes.
    pub signature: Option<String>,
    /// Set once a page comes back empty: there is nothing older, so the descent
    /// has reached the beginning of this program's history.
    ///
    /// Stored rather than recomputed, because "no page came back" and "we have
    /// not asked yet" are indistinguishable from the cursor alone — and a
    /// backfill that cannot tell them apart either restarts from the tip on every
    /// boot or stops one page early and calls the history complete.
    pub complete: bool,
}

/// A source of transaction logs for one program.
pub trait LogSource {
    type Error: std::fmt::Debug;

    /// Transactions after `after`, in ledger order, up to `limit`.
    ///
    /// Must return the source's **current** view, not a diff. The ingestor
    /// re-reads the unfinalised range every poll precisely so that a rollback
    /// shows up as a disagreement rather than as silence.
    fn fetch(&mut self, after: &Cursor, limit: usize) -> Result<Vec<TransactionLogs>, Self::Error>;

    /// The highest slot the cluster considers final.
    ///
    /// Below this, history does not change. Above it, anything may still be
    /// rolled back — which is why the ingestor keeps two projections.
    fn finalized_slot(&mut self) -> Result<u64, Self::Error>;
}

/// One entry of a page, identified enough to bound the next one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageBound {
    pub slot: u64,
    pub signature: String,
}

/// What a page of a descent spanned, failures and all.
///
/// Both ends, because they are needed for different things and a page always has
/// both or neither — which is why they live in one `Option` rather than two.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageRange {
    /// The exclusive upper bound of the *next* page.
    pub oldest: PageBound,
    /// The newest entry the page covered. Used only to check that the source
    /// honoured the bound it was given — a page reaching above it means the
    /// source is not descending, and every subsequent step would ask the same
    /// question and get the same answer.
    pub newest: PageBound,
}

/// One page of a descent.
///
/// The two fields are separate because they answer different questions, and
/// conflating them is a real bug rather than a tidiness point. "What should I
/// fold" excludes transactions that failed — their writes were rolled back, so
/// their events did not happen, even though the events are still in the log.
/// "Where do I ask from next" must not, because a page consisting *entirely* of
/// failed transactions is an ordinary thing for a spammed program to have, and a
/// descent that read `transactions.is_empty()` as genesis would stop there and
/// report the remaining history as complete.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DescentPage {
    /// Worth folding, oldest first. May be empty while `covered` is `Some`.
    pub transactions: Vec<TransactionLogs>,
    /// What the page spanned. `None` means there was nothing older to read at
    /// all, which is the one and only end condition.
    pub covered: Option<PageRange>,
}

/// A source that can also be walked backwards, toward genesis.
///
/// Separate from [`LogSource`] rather than another method on it, because the two
/// traversals are not variations of one thing. A live poll asks "what is new",
/// must re-read its own tail so a rollback shows up, and never terminates. A
/// descent asks "what came before", reads only history that is already settled,
/// and finishes. A webhook subscription is a perfectly good `LogSource` and
/// cannot implement this at all.
pub trait DescendingSource: LogSource {
    /// One page strictly older than `descent`, up to `limit` entries.
    ///
    /// Transactions come back oldest-first *within the page*, the same contract
    /// [`LogSource::fetch`] has — even though consecutive pages arrive
    /// progressively older. That inversion is the whole difficulty of a backfill
    /// and is dealt with in [`crate::ingest::Backfill`], not here.
    fn fetch_before(&mut self, descent: &Descent, limit: usize)
        -> Result<DescentPage, Self::Error>;
}

/// A source backed by an in-memory ledger the test can rewrite.
///
/// Lives outside `#[cfg(test)]` so the integration crate can drive it too.
#[derive(Debug, Default)]
pub struct ScriptedSource {
    pub ledger: Vec<TransactionLogs>,
    pub finalized: u64,
    /// Counts `fetch` calls, so a test can assert the ingestor is not re-reading
    /// history it has already finalised.
    pub fetches: usize,
}

impl ScriptedSource {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a transaction at `slot`.
    pub fn push(&mut self, signature: &str, slot: u64, logs: Vec<String>) -> &mut Self {
        self.ledger.push(TransactionLogs {
            signature: signature.to_owned(),
            slot,
            logs,
        });
        self
    }

    /// Drops everything above `slot`, as a rollback would.
    pub fn roll_back_to(&mut self, slot: u64) -> &mut Self {
        self.ledger.retain(|tx| tx.slot <= slot);
        self
    }

    pub fn finalize_through(&mut self, slot: u64) -> &mut Self {
        self.finalized = slot;
        self
    }
}

impl LogSource for ScriptedSource {
    type Error = std::convert::Infallible;

    fn fetch(&mut self, after: &Cursor, limit: usize) -> Result<Vec<TransactionLogs>, Self::Error> {
        self.fetches += 1;

        // Resume immediately after the named signature, wherever it now sits.
        // Looking it up by name rather than by index is what makes a cursor
        // survive a ledger rewritten underneath it.
        //
        // When the signature is *not* found, this deliberately restarts from the
        // beginning rather than returning nothing. That mirrors what an RPC
        // provider does with an unknown `until`: it has no idea which slot the
        // caller considers settled, so it cannot filter to "after" anything and
        // simply serves history. Handing back the caller's own past is the
        // symptom of a cursor from a different ledger, and the ingestor is
        // supposed to notice — an earlier version of this fake filtered on
        // `slot >= after.slot`, which hid exactly that case.
        let start = match &after.signature {
            Some(signature) => self
                .ledger
                .iter()
                .position(|tx| &tx.signature == signature)
                .map(|i| i + 1)
                .unwrap_or(0),
            None => 0,
        };

        Ok(self
            .ledger
            .iter()
            .skip(start)
            .take(limit)
            .cloned()
            .collect())
    }

    fn finalized_slot(&mut self) -> Result<u64, Self::Error> {
        Ok(self.finalized)
    }
}

impl DescendingSource for ScriptedSource {
    fn fetch_before(
        &mut self,
        descent: &Descent,
        limit: usize,
    ) -> Result<DescentPage, Self::Error> {
        self.fetches += 1;

        // The exclusive upper bound, located by name for the same reason `fetch`
        // resumes by name: a ledger rewritten underneath a stored bound must not
        // silently become a different range.
        //
        // Unlike `fetch`, an unknown signature here restarts from the *tip*
        // rather than from the beginning — which for a descent is the same
        // stance, "serve history from the top", and the same symptom of a bound
        // belonging to another ledger.
        let end = match &descent.signature {
            Some(signature) => self
                .ledger
                .iter()
                .position(|tx| &tx.signature == signature)
                .unwrap_or(self.ledger.len()),
            None => self.ledger.len(),
        };

        // The newest `limit` of what remains below the bound, handed back
        // oldest-first.
        let start = end.saturating_sub(limit);
        let window = &self.ledger[start..end];

        let page_bound = |tx: &TransactionLogs| PageBound {
            slot: tx.slot,
            signature: tx.signature.clone(),
        };

        Ok(DescentPage {
            transactions: window.to_vec(),
            covered: match (window.first(), window.last()) {
                (Some(oldest), Some(newest)) => Some(PageRange {
                    oldest: page_bound(oldest),
                    newest: page_bound(newest),
                }),
                _ => None,
            },
        })
    }
}
