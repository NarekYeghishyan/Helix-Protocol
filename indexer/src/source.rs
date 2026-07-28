//! Where transaction logs come from, and how far the chain has committed to them.
//!
//! A trait rather than an RPC client, for the same reason the decoder is a pure
//! function: the interesting failures — rollbacks, gaps, redelivery, resuming
//! after a crash — are all expressible without a network, and none of them can be
//! *tested* with one. A scripted source can roll a slot back on demand; devnet
//! cannot be asked to.
//!
//! The RPC implementation is Phase 4.1 remainder. Everything that decides what to
//! do with what a source returns lives in [`crate::ingest`], which is testable
//! today.

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
