//! Reconstructs Helix protocol state from the events the programs emit.
//!
//! Every state transition in the four programs emits an Anchor event carrying an
//! on-chain timestamp, so history is reconstructable from logs without polling
//! account state. This crate is the part that does the reconstructing: decode,
//! attribute, fold.
//!
//! **Everything compiled by default is pure.** I/O exists in two modules — [`rpc`]
//! reads the chain, `store` writes to Postgres — and both are behind features that
//! are off, so the default build has neither a socket nor a database driver in it,
//! and the parts where analytics bugs actually live are testable without either.
//! That is the separation the whole crate is arranged around: ingestion and
//! persistence are what cannot be verified offline, so each is made as small as it
//! can be and pushed to one edge rather than threaded through the fold.
//!
//! # Layers
//!
//! | Module | Responsibility |
//! |---|---|
//! | [`event`] | The 38 event types, and decoding one from its wire form |
//! | [`logs`] | Attributing `Program data:` lines to the invocation that emitted them |
//! | [`projection`] | Folding events into queryable state, exactly once each |
//! | [`ingest`] | Driving a source into two projections, safely across reorgs |
//! | `rpc` | The one module that opens a socket, behind the `rpc` feature |
//! | `store` | Persisting what finalised, behind the `postgres` feature |
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
#[cfg(feature = "rpc")]
pub mod rpc;
#[cfg(feature = "server")]
pub mod server;
pub mod source;
#[cfg(feature = "postgres")]
pub mod store;

pub use api::{Api, Finality};
pub use event::{HelixEvent, Program};
pub use ingest::{IngestError, Ingestor, PollOutcome, SettledTransaction};
pub use logs::{parse, Anomaly, EmittedEvent, ParsedLogs};
pub use projection::{Analytics, EventId};
#[cfg(feature = "rpc")]
pub use rpc::{Commitment, RpcError, RpcLogSource};
pub use source::{Cursor, LogSource, TransactionLogs};
#[cfg(feature = "postgres")]
pub use store::{Restored, Store, StoreError};
