//! A [`LogSource`] backed by a real cluster's JSON-RPC endpoint.
//!
//! Everything that *decides what to do* with logs lives in [`crate::ingest`] and
//! is tested against a scripted source. This module is the other half: the part
//! that can only be wrong in ways a cluster demonstrates, kept small and kept
//! separate so the untestable surface is as thin as it can be.
//!
//! # Why this speaks JSON-RPC directly
//!
//! Not for the fun of it. `solana-rpc-client` is published at 4.2.0-rc while this
//! workspace resolves the Solana crates at 3.x through `anchor-lang` 1.1.2, and
//! the graph already carries eight duplicated `solana-*` crates. Adding a second
//! major version of the SDK is the same trade that ruled out Trident, and it buys
//! typed wrappers around three read methods that carry no signatures and no
//! account decoding. The whole surface used here is:
//!
//! | Method | For |
//! |---|---|
//! | `getSignaturesForAddress` | which transactions touched a program |
//! | `getTransaction` | the logs of one |
//! | `getSlot` (`finalized`) | the watermark below which history is settled |
//!
//! Re-check when the SDK versions converge; this becomes a thin adapter then.
//!
//! # What a real endpoint does that a fake would not have taught us
//!
//! Four things, each of which is a silent wrong answer rather than an error:
//!
//! - **`getSignaturesForAddress` returns newest-first.** [`LogSource::fetch`]
//!   is specified in ledger order. Handing the RPC's order straight through
//!   folds every batch backwards, which for running totals means the projection
//!   settles on the *oldest* value in the batch rather than the newest.
//! - **`limit` counts from the newest end.** So "give me the 100 after my
//!   cursor" is not a request this API can express: asking for 100 with `until`
//!   set returns the 100 *newest*, and if more than 100 arrived meanwhile the
//!   gap in the middle is never mentioned again. [`RpcLogSource`] therefore
//!   descends to the cursor and reverses, and refuses rather than guesses when
//!   the cursor is beyond [`RpcLogSource::max_scan`].
//! - **Failed transactions appear in the signature list.** Their writes were
//!   rolled back, so their events did not happen — but the events are still in
//!   the log, and folding them credits stakes that never landed.
//! - **`logMessages` is nullable.** A node with transaction logs disabled
//!   answers every query successfully and reports that nothing was emitted.
//!
//! The first three are asserted in `tests/integration/tests/rpc_source_live.rs`
//! against a validator; the fourth is asserted here, since it is a decoding
//! question rather than a cluster one.

use anchor_lang::prelude::Pubkey;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

use crate::event::Program;
use crate::source::{Cursor, LogSource, TransactionLogs};

/// How far the cluster has committed to a slot.
///
/// `fetch` reads at `Confirmed` deliberately: waiting for finality before
/// showing anything costs ~13 seconds of staleness on every panel, and the
/// two-projection design in [`crate::ingest`] exists precisely so that
/// unfinalised data can be shown *and* taken back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Commitment {
    Confirmed,
    Finalized,
}

impl Commitment {
    fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::Finalized => "finalized",
        }
    }
}

#[derive(Debug)]
pub enum RpcError {
    /// The request never got an answer.
    Transport(String),
    /// The node answered with a JSON-RPC error object.
    Rpc { code: i64, message: String },
    /// The node answered with something this client cannot read. Distinct from
    /// [`Self::Rpc`] because it means the client and the node disagree about the
    /// protocol, not that the request was bad.
    Malformed(String),
    /// The cursor is further behind the tip than `max_scan` signatures.
    ///
    /// An error rather than a partial answer. The batch that *could* be returned
    /// is the newest chunk, which is not contiguous with the cursor, and folding
    /// it would leave a hole no later poll ever revisits — the projection would
    /// simply be missing transactions, with every number still looking ordinary.
    /// The recovery is a backfill, which is a different traversal.
    FellBehind { address: Pubkey, scanned: usize },
    /// A transaction is known to have touched the program, and its logs cannot
    /// be read.
    ///
    /// Also an error, for the reason in the module docs: the alternative is
    /// recording that a transaction emitted nothing, which is indistinguishable
    /// from a transaction that really did. Nodes run with `--no-transaction-logs`
    /// or with a pruned ledger produce this, and the fix is to ask a different
    /// node rather than to accept the gap.
    LogsUnavailable { signature: String },
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(e) => write!(f, "transport: {e}"),
            Self::Rpc { code, message } => write!(f, "rpc error {code}: {message}"),
            Self::Malformed(e) => write!(f, "malformed response: {e}"),
            Self::FellBehind { address, scanned } => write!(
                f,
                "cursor is more than {scanned} signatures behind the tip for {address}; \
                 the gap needs a backfill, not a poll"
            ),
            Self::LogsUnavailable { signature } => write!(
                f,
                "no logs available for {signature}; the node is not recording them, \
                 so what this transaction did cannot be established"
            ),
        }
    }
}

impl std::error::Error for RpcError {}

/// One entry of `getSignaturesForAddress`.
#[derive(Clone, Debug, Deserialize)]
struct SignatureInfo {
    signature: String,
    slot: u64,
    /// `null` when the transaction succeeded. Anything else and its writes were
    /// rolled back.
    #[serde(default)]
    err: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct TransactionMeta {
    /// Nullable, and the reason [`RpcError::LogsUnavailable`] exists.
    #[serde(rename = "logMessages")]
    log_messages: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct TransactionResponse {
    slot: u64,
    meta: Option<TransactionMeta>,
}

/// Follows a set of program addresses on one cluster.
pub struct RpcLogSource {
    url: String,
    /// Deliberately plural. A governance proposal executing a treasury transfer
    /// is one transaction recorded under both addresses, and following only the
    /// entry points would miss any transaction that reaches a program purely by
    /// CPI. Duplicates are removed by signature, which is safe because the log
    /// parser attributes events by invoke depth rather than by which address was
    /// queried — the same log yields the same events either way.
    ///
    /// Removing that dedup is caught by
    /// `rpc_source_live::a_transaction_reported_by_three_addresses_is_folded_once`.
    addresses: Vec<Pubkey>,
    agent: ureq::Agent,
    commitment: Commitment,
    /// The largest number of signatures a single `fetch` will walk back through
    /// before giving up. Bounds the work one poll can do.
    max_scan: usize,
    /// Signatures per `getSignaturesForAddress` call. The RPC caps this at 1000.
    page: usize,
    next_id: u64,
    /// Requests issued, for the operator to see. An indexer whose RPC bill is a
    /// surprise is an indexer nobody keeps running.
    requests: usize,
}

impl RpcLogSource {
    /// Follows every program in [`Program::ALL`].
    pub fn new(url: impl Into<String>) -> Self {
        Self::for_addresses(url, Program::ALL.iter().map(|p| p.id()).collect())
    }

    pub fn for_addresses(url: impl Into<String>, addresses: Vec<Pubkey>) -> Self {
        Self {
            url: url.into(),
            addresses,
            agent: ureq::Agent::new_with_config(
                ureq::Agent::config_builder()
                    .timeout_global(Some(Duration::from_secs(30)))
                    .build(),
            ),
            commitment: Commitment::Confirmed,
            max_scan: 10_000,
            page: 1_000,
            next_id: 1,
            requests: 0,
        }
    }

    pub fn with_commitment(mut self, commitment: Commitment) -> Self {
        self.commitment = commitment;
        self
    }

    pub fn with_max_scan(mut self, max_scan: usize) -> Self {
        self.max_scan = max_scan;
        self
    }

    /// Lowered by the live tests so the paging path is exercised over a ledger
    /// small enough to build in a test, rather than only over one big enough to
    /// need it.
    pub fn with_page_size(mut self, page: usize) -> Self {
        self.page = page.clamp(1, 1_000);
        self
    }

    pub fn requests(&self) -> usize {
        self.requests
    }

    fn call<T: DeserializeOwned>(&mut self, method: &str, params: Value) -> Result<T, RpcError> {
        self.next_id += 1;
        self.requests += 1;
        let body = json!({
            "jsonrpc": "2.0",
            "id": self.next_id,
            "method": method,
            "params": params,
        });

        let mut response = self
            .agent
            .post(&self.url)
            .send_json(&body)
            .map_err(|e| RpcError::Transport(e.to_string()))?;

        let envelope: Value = response
            .body_mut()
            .read_json()
            .map_err(|e| RpcError::Malformed(e.to_string()))?;

        Self::unwrap_envelope(envelope)
    }

    /// Pulls `result` out of one JSON-RPC envelope.
    ///
    /// Split out so the error paths can be tested without a node: a JSON-RPC
    /// error is transported over HTTP 200, so a client that only checks the
    /// status code reports success for every failure the node reports.
    fn unwrap_envelope<T: DeserializeOwned>(envelope: Value) -> Result<T, RpcError> {
        if let Some(error) = envelope.get("error") {
            return Err(RpcError::Rpc {
                code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
                message: error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("(no message)")
                    .to_owned(),
            });
        }

        let result = envelope
            .get("result")
            .ok_or_else(|| RpcError::Malformed("no result and no error".into()))?;

        serde_json::from_value(result.clone()).map_err(|e| RpcError::Malformed(e.to_string()))
    }

    /// Every successful signature for `address` at or above `min_slot`, in
    /// ledger order.
    ///
    /// Walks *backwards* from the tip, because that is the only direction this
    /// API offers: `limit` counts from the newest end, so asking for the oldest
    /// N after a cursor is not expressible.
    ///
    /// # Why the stop condition is a slot and not the cursor's signature
    ///
    /// `getSignaturesForAddress` takes an `until` signature, which looks like
    /// the natural way to say "everything after where I got to". The worry with
    /// it here is that the ingestor's cursor is one signature for the whole
    /// protocol while this walks four addresses, so the cursor names a
    /// transaction that touched only one of them — and an `until` the other
    /// three have never seen might not stop them.
    ///
    /// Measured against Agave 3.1.10, it does stop them: `until` is resolved to
    /// the signature's slot *globally* rather than within the address's own
    /// history, so an unrelated signature still bounds the walk, and a signature
    /// that is in the history is exclusive at signature granularity with its
    /// same-slot siblings preserved. Both implementations are correct, and the
    /// mutation test in `docs/TESTING.md` records that swapping this back does
    /// not fail anything — because it is not a bug.
    ///
    /// A slot is used anyway, for two reasons that hold without measuring
    /// anything:
    ///
    /// - **`until` needs the node to still hold that signature.** Resolving it
    ///   is a lookup in the transaction-status index, which is pruned — on a
    ///   local validator that window is minutes. A cursor older than the window
    ///   cannot be resolved, and what a node does then is not specified.
    /// - **The slot form is what makes mid-slot resumption expressible.** The
    ///   walk keeps the cursor's own slot (`>= min_slot`) so that a batch cut
    ///   off by `limit` in the middle of one can be finished on the next poll;
    ///   [`LogSource::fetch`] then drops what was already applied. `until` is
    ///   exclusive, so the same-slot remainder would have to be recovered some
    ///   other way.
    fn signatures_after(
        &mut self,
        address: Pubkey,
        min_slot: u64,
    ) -> Result<Vec<SignatureInfo>, RpcError> {
        let mut newest_first: Vec<SignatureInfo> = Vec::new();
        let mut before: Option<String> = None;

        loop {
            let mut options = json!({
                "limit": self.page,
                "commitment": self.commitment.as_str(),
            });
            if let Some(before) = &before {
                options["before"] = json!(before);
            }

            let page: Vec<SignatureInfo> = self.call(
                "getSignaturesForAddress",
                json!([address.to_string(), options]),
            )?;

            // Genesis reached: there is nothing older to ask for.
            let Some(oldest) = page.last() else { break };
            let (descended_past_cursor, short) = (oldest.slot < min_slot, page.len() < self.page);

            // `before` for the next descent is the oldest of this page, taken
            // before the failed ones are filtered out — skipping a failure would
            // make the next page start from the wrong place and repeat
            // everything between.
            before = Some(oldest.signature.clone());
            newest_first.extend(page);

            if descended_past_cursor || short {
                break;
            }
            if newest_first.len() > self.max_scan {
                return Err(RpcError::FellBehind {
                    address,
                    scanned: self.max_scan,
                });
            }
        }

        // A rolled-back transaction is in the ledger and did nothing. Its events
        // are still in its log, so this filter is load-bearing rather than an
        // optimisation — though it is also the cheaper place to do it, since a
        // signature dropped here is a `getTransaction` never issued.
        newest_first.retain(|s| s.err.is_none() && s.slot >= min_slot);
        newest_first.reverse();
        Ok(newest_first)
    }

    /// The logs of each signature, in the order given.
    ///
    /// One JSON-RPC batch rather than one request per signature. A thousand
    /// round trips to build one page is the difference between an indexer that
    /// keeps up with a chain and one that does not.
    fn logs_for(&mut self, signatures: &[SignatureInfo]) -> Result<Vec<TransactionLogs>, RpcError> {
        if signatures.is_empty() {
            return Ok(Vec::new());
        }

        let batch: Vec<Value> = signatures
            .iter()
            .map(|s| {
                self.next_id += 1;
                json!({
                    "jsonrpc": "2.0",
                    "id": self.next_id,
                    "method": "getTransaction",
                    "params": [s.signature, {
                        "encoding": "json",
                        "commitment": self.commitment.as_str(),
                        // Without this the node refuses every versioned
                        // transaction outright, which on a modern cluster is
                        // most of them.
                        "maxSupportedTransactionVersion": 0,
                    }],
                })
            })
            .collect();

        self.requests += 1;
        let mut response = self
            .agent
            .post(&self.url)
            .send_json(Value::Array(batch))
            .map_err(|e| RpcError::Transport(e.to_string()))?;

        let envelopes: Vec<Value> = response
            .body_mut()
            .read_json()
            .map_err(|e| RpcError::Malformed(e.to_string()))?;

        if envelopes.len() != signatures.len() {
            return Err(RpcError::Malformed(format!(
                "asked for {} transactions and got {}",
                signatures.len(),
                envelopes.len()
            )));
        }

        // A JSON-RPC batch response may come back in any order, and the spec
        // says so. Pairing by position is the bug that produces a projection
        // where every transaction's logs belong to a different transaction, and
        // it would pass every test against a node that happens to answer in
        // order. Pair by id instead.
        let mut by_id: std::collections::HashMap<u64, Value> = envelopes
            .into_iter()
            .filter_map(|e| e.get("id").and_then(Value::as_u64).map(|id| (id, e)))
            .collect();

        let first_id = self.next_id - signatures.len() as u64 + 1;
        signatures
            .iter()
            .enumerate()
            .map(|(i, signature)| {
                let envelope = by_id.remove(&(first_id + i as u64)).ok_or_else(|| {
                    RpcError::Malformed(format!("no response for {}", signature.signature))
                })?;
                let transaction: Option<TransactionResponse> = Self::unwrap_envelope(envelope)?;
                let Some(transaction) = transaction else {
                    // The node listed the signature and then had no record of
                    // it. Same class of gap as a missing log, and the same
                    // stance: refuse rather than record that it did nothing.
                    return Err(RpcError::LogsUnavailable {
                        signature: signature.signature.clone(),
                    });
                };

                let logs = transaction
                    .meta
                    .and_then(|m| m.log_messages)
                    .ok_or_else(|| RpcError::LogsUnavailable {
                        signature: signature.signature.clone(),
                    })?;

                Ok(TransactionLogs {
                    signature: signature.signature.clone(),
                    // The transaction's own record of where it landed, not the
                    // slot the signature index reported a moment earlier. The
                    // two are read by separate calls, so a rollback between them
                    // shows up here — and slot is what drives promotion past the
                    // finality watermark, so taking the staler of the two would
                    // finalise a transaction at a slot it is no longer in.
                    slot: transaction.slot,
                    logs,
                })
            })
            .collect()
    }
}

impl LogSource for RpcLogSource {
    type Error = RpcError;

    fn fetch(&mut self, after: &Cursor, limit: usize) -> Result<Vec<TransactionLogs>, Self::Error> {
        let mut merged: Vec<SignatureInfo> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

        for address in self.addresses.clone() {
            for info in self.signatures_after(address, after.slot)? {
                if seen.insert(info.signature.clone()) {
                    merged.push(info);
                }
            }
        }

        // Stable, so each address's ledger order survives inside a slot. Two
        // addresses' transactions in one slot interleave in address order, which
        // is arbitrary but *fixed* — and fixed is the property that matters,
        // because the ingestor detects rollbacks by comparing this sequence
        // against the prefix it already applied. An ordering that varied between
        // polls would look exactly like a fork.
        merged.sort_by_key(|s| s.slot);

        // The cursor's own slot is re-read every poll, because a batch cut off
        // by `limit` can end in the middle of one. Everything up to and
        // including the cursor's signature has been applied, so it is dropped
        // here; a cursor whose signature is *not* in this slot is one that
        // pointed at another address's transaction, and re-serving the slot is
        // then correct — the projection is idempotent, and the alternative is
        // skipping whatever shared the slot with it.
        if let Some(signature) = &after.signature {
            if let Some(applied) = merged.iter().position(|s| &s.signature == signature) {
                merged.drain(..=applied);
            }
        }

        merged.truncate(limit);
        self.logs_for(&merged)
    }

    fn finalized_slot(&mut self) -> Result<u64, Self::Error> {
        self.call(
            "getSlot",
            json!([{ "commitment": Commitment::Finalized.as_str() }]),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_json_rpc_error_is_an_error_despite_the_http_200() {
        let envelope = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": { "code": -32602, "message": "Invalid param" },
        });
        let err = RpcLogSource::unwrap_envelope::<u64>(envelope)
            .expect_err("a JSON-RPC error object was read as success");
        assert!(matches!(err, RpcError::Rpc { code: -32602, .. }));
    }

    /// The nullable field that makes silence look like an answer.
    #[test]
    fn a_transaction_without_logs_is_not_a_transaction_that_did_nothing() {
        let envelope = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": { "slot": 42, "meta": { "logMessages": null } },
        });
        let decoded: Option<TransactionResponse> =
            RpcLogSource::unwrap_envelope(envelope).expect("envelope");
        assert!(
            decoded
                .expect("a transaction")
                .meta
                .expect("meta")
                .log_messages
                .is_none(),
            "a null logMessages decoded as an empty log, which reads as 'emitted nothing'"
        );
    }

    /// `getTransaction` answers `null` for a signature the node has pruned.
    #[test]
    fn an_unknown_signature_decodes_as_absent_rather_than_failing_to_parse() {
        let envelope = json!({ "jsonrpc": "2.0", "id": 1, "result": null });
        let decoded: Option<TransactionResponse> =
            RpcLogSource::unwrap_envelope(envelope).expect("null is a valid result");
        assert!(decoded.is_none());
    }

    #[test]
    fn every_program_is_followed_by_default() {
        let source = RpcLogSource::new("http://127.0.0.1:8899");
        assert_eq!(source.addresses.len(), Program::ALL.len());
        for program in Program::ALL {
            assert!(
                source.addresses.contains(&program.id()),
                "{} is not followed",
                program.name()
            );
        }
    }
}
