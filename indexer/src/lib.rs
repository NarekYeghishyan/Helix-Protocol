//! Reconstructs Helix protocol state from the events the programs emit.
//!
//! Every state transition in the four programs emits an Anchor event carrying an
//! on-chain timestamp, so history is reconstructable from logs without polling
//! account state. This crate is the part that does the reconstructing: decode,
//! attribute, fold. What it deliberately is *not* is a daemon — there is no RPC
//! client, no database driver and no network I/O anywhere in it.
//!
//! That separation is the point. Ingestion is the part that cannot be tested
//! without a cluster; decoding and folding are the parts where the bugs that
//! corrupt analytics actually live, and keeping them pure means they can be
//! tested against the real programs today, before anything is deployed.
//!
//! # Layers
//!
//! | Module | Responsibility |
//! |---|---|
//! | [`event`] | The 34 event types, and decoding one from its wire form |
//! | [`logs`] | Attributing `Program data:` lines to the invocation that emitted them |
//! | [`projection`] | Folding events into queryable state, exactly once each |
//!
//! # How it is verified
//!
//! `tests/integration/tests/indexer_reconciliation.rs` runs real transactions
//! against the real BPF programs, captures the logs the runtime produced, folds
//! them through this crate, and asserts the result matches the on-chain accounts
//! field by field. An analytics stack that claims to match the chain and cannot
//! demonstrate it is a rumour.
//!
//! # Delivery guarantees this assumes
//!
//! **At-least-once, out of nothing.** Confirmed logs can be redelivered, a
//! backfill overlaps a live stream, and a reorg makes both happen at once.
//! [`projection::Analytics::apply_transaction`] is therefore idempotent on
//! `(signature, log_index)` — see [`projection::EventId`] for why the log index
//! is not optional.
//!
//! **Silence is never assumed to mean nothing happened.** A truncated log or an
//! undecodable payload is surfaced as [`logs::Anomaly`] rather than skipped. An
//! indexer that drops what it cannot read reports wrong numbers precisely when
//! something unusual happened, which is exactly when someone is looking at them.

pub mod api;
pub mod event;
pub mod ingest;
pub mod logs;
pub mod projection;
#[cfg(feature = "server")]
pub mod server;
pub mod source;

pub use api::{Api, Finality};
pub use event::{HelixEvent, Program};
pub use ingest::{IngestError, Ingestor, PollOutcome};
pub use logs::{parse, Anomaly, EmittedEvent, ParsedLogs};
pub use projection::{Analytics, EventId};
pub use source::{Cursor, LogSource, TransactionLogs};
