//! Making the projection survive a restart.
//!
//! Everything else in this crate is a function of its input. This module is the
//! one place where the answer depends on what happened in a previous process, so
//! it is the one place where "it worked when I ran it" is not evidence of
//! anything. What it has to guarantee, in the order the guarantees matter:
//!
//! 1. **The cursor never gets ahead of the rows.** If a crash can leave the
//!    cursor at slot 900 with slot 900's events unwritten, the next run resumes
//!    at 901 and those events are lost — permanently, silently, and only for the
//!    transactions straddling a restart. Every write in [`Store::commit`] is
//!    therefore in one database transaction with the cursor update, so the pair
//!    is atomic and a torn write is impossible rather than unlikely.
//!
//! 2. **Only finalised state is written.** A row a fork can revoke is a number
//!    that was never true. [`crate::ingest::SettledTransaction`] is the boundary:
//!    the ingestor hands over transactions at the moment they finalise, and the
//!    unfinalised tail — tens of slots, seconds of chain — is re-read from the
//!    source on restart rather than persisted.
//!
//! 3. **Replay changes nothing.** Redelivery is routine, so `commit` must be
//!    safe to run twice with the same batch. Inserts are `ON CONFLICT DO
//!    NOTHING`; upserts assign rather than accumulate, and are guarded on
//!    `updated_at_slot` so a backfill replaying an old slot cannot overwrite a
//!    newer live update.
//!
//! 4. **A loaded projection is the one that was saved.** Including the part that
//!    is not a number: see [`Store::load`] and the applied set.
//!
//! # What it deliberately is not
//!
//! Not a migration system, not a connection pool, and not async. [`Store::migrate`]
//! creates what is missing and refuses a schema version it does not know; it will
//! not alter an existing table. One connection, because one indexer writes — the
//! ordering guarantees above are between a cursor and its rows, not between
//! writers. And blocking, because [`crate::source::LogSource`] is blocking: an
//! async driver here would mean an executor threaded through a crate whose other
//! I/O is one HTTP call.
//!
//! # u64 does not fit in a BIGINT
//!
//! Postgres has no unsigned types, and `BIGINT` is signed 64-bit — a token balance
//! above 2^63 overflows it, which for a 9-decimal mint is 9.2 billion whole
//! tokens. `schema.sql` uses `NUMERIC(20, 0)` for every amount.
//!
//! The Rust driver has the mirror image of that problem: `postgres` maps `BIGINT`
//! to `i64` and `NUMERIC` to nothing at all without a decimal crate, so the
//! obvious binding casts the u64 to i64 on the way in and reintroduces exactly the
//! overflow the column type exists to prevent. Every amount here therefore crosses
//! as text and is cast in SQL — `$1::text::numeric` going in, `column::text`
//! coming out — which is the same decision, made for the same reason, as the read
//! API serialising amounts as JSON strings.

use anchor_lang::prelude::Pubkey;
use postgres::types::ToSql;
use postgres::{Client, NoTls, Transaction};
use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr as _;

use crate::event::HelixEvent;
use crate::ingest::SettledTransaction;
use crate::logs::Anomaly;
use crate::projection::{
    Analytics, EventId, PoolStats, PositionStats, ProposalStats, RealmStats, StreamStats,
    TreasuryStats,
};
use crate::source::Cursor;

/// The schema `migrate` writes and `load` expects.
///
/// Bumped when `schema.sql` changes shape. A database stamped with anything else
/// is refused rather than written to: the failure this prevents is two binaries
/// at different versions sharing one database, where the older one writes rows
/// with columns it has never heard of silently left at their defaults.
const SCHEMA_VERSION: i32 = 1;

/// The cursor this binding maintains. The schema allows a second — a backfill
/// running downwards — which nothing writes yet; see ROADMAP 4.3.
const LIVE_CURSOR: &str = "live";

#[derive(Debug)]
pub enum StoreError {
    Connect(postgres::Error),
    Query {
        what: &'static str,
        source: postgres::Error,
    },
    /// The database was created by a different version of this schema.
    SchemaVersion {
        found: i32,
        expected: i32,
    },
    /// A row that cannot be read back into the type it was written from.
    ///
    /// Always a bug or a hand-edited database, never a transient condition, so it
    /// stops the load rather than skipping the row — a projection silently short
    /// one pool is worse than one that refuses to start.
    Malformed {
        table: &'static str,
        column: &'static str,
        value: String,
    },
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connect(e) => write!(f, "cannot connect: {e}"),
            Self::Query { what, source } => write!(f, "{what}: {source}"),
            Self::SchemaVersion { found, expected } => write!(
                f,
                "database is at schema version {found}, this build writes {expected} — \
                 point it at a fresh database or migrate it by hand"
            ),
            Self::Malformed {
                table,
                column,
                value,
            } => write!(f, "{table}.{column} does not parse: {value:?}"),
        }
    }
}

impl std::error::Error for StoreError {}

/// Everything one process needs to carry on where another stopped.
///
/// The two halves come from a single read for a reason — see [`Store::load`].
pub struct Restored {
    pub cursor: Cursor,
    pub state: Analytics,
}

pub struct Store {
    client: Client,
}

impl Store {
    /// Connects with a libpq-style URL, e.g. `postgres://user:pw@host:5432/helix`.
    ///
    /// `NoTls`. A production deployment puts the database behind TLS and this
    /// would take a connector; it is left out rather than stubbed, because a
    /// binding that accepts `sslmode=require` and quietly ignores it is worse than
    /// one that never claimed to.
    pub fn connect(url: &str) -> Result<Self, StoreError> {
        let client = Client::connect(url, NoTls).map_err(StoreError::Connect)?;
        Ok(Self { client })
    }

    /// Creates the schema if it is absent, and checks the version if it is not.
    ///
    /// The SQL is `schema.sql` itself, included at compile time, so the schema the
    /// tests run against is the file that documents it rather than a second copy
    /// that resembles it.
    pub fn migrate(&mut self) -> Result<(), StoreError> {
        self.client
            .batch_execute(include_str!("../sql/schema.sql"))
            .map_err(|source| StoreError::Query {
                what: "applying schema.sql",
                source,
            })?;

        let found: i32 = self
            .client
            .query_one("SELECT version FROM schema_version", &[])
            .map_err(|source| StoreError::Query {
                what: "reading schema_version",
                source,
            })?
            .get(0);

        if found != SCHEMA_VERSION {
            return Err(StoreError::SchemaVersion {
                found,
                expected: SCHEMA_VERSION,
            });
        }
        Ok(())
    }

    /// Writes one poll's finalised transactions and the cursor, atomically.
    ///
    /// `state` is the projection *after* folding `settled` — the store does not
    /// fold, it records. That is what keeps the persisted numbers identical to the
    /// in-memory ones by construction rather than by two implementations agreeing.
    ///
    /// Returns the number of event rows that were new.
    ///
    /// Nothing is written when `settled` is empty, including the cursor: a poll
    /// that finalised nothing has not moved the cursor either, and an `UPDATE`
    /// setting a row to its own value is a lock taken for no reason.
    pub fn commit(
        &mut self,
        cursor: &Cursor,
        state: &Analytics,
        settled: &[SettledTransaction],
    ) -> Result<usize, StoreError> {
        if settled.is_empty() {
            return Ok(0);
        }

        let touched = Touched::of(settled);
        let mut tx = self
            .client
            .transaction()
            .map_err(|source| StoreError::Query {
                what: "beginning a transaction",
                source,
            })?;

        let inserted = write_events(&mut tx, settled)?;
        write_anomalies(&mut tx, settled, state)?;

        // Parents before children: `positions` references `pools`, `streams`
        // references `treasuries`, `votes` references `proposals`. Within one
        // transaction Postgres checks foreign keys per statement, so the order is
        // not cosmetic.
        write_realms(&mut tx, state, &touched)?;
        write_pools(&mut tx, state, &touched)?;
        write_treasuries(&mut tx, state, &touched)?;
        write_proposals(&mut tx, state, &touched)?;
        write_positions(&mut tx, state, &touched)?;
        write_streams(&mut tx, state, &touched)?;
        write_votes(&mut tx, &touched)?;
        write_cursor(&mut tx, cursor)?;

        // The commit is what makes the cursor and the rows one fact. Everything
        // above is invisible until this line returns, so a crash anywhere in it
        // leaves the previous cursor and the previous rows — consistent, and
        // behind rather than corrupt.
        tx.commit().map_err(|source| StoreError::Query {
            what: "committing",
            source,
        })?;

        Ok(inserted)
    }

    /// Reads back the cursor and the projection it describes.
    ///
    /// **One snapshot, both halves.** They are returned together rather than
    /// through two calls because a cursor from one moment and a projection from
    /// another is the one combination that fails silently: resume at a slot whose
    /// events are not all in the projection and the ingestor will never fetch them
    /// again, because as far as it is concerned they are behind it.
    ///
    /// The projection carries the applied set for every event at or above the
    /// cursor's slot, which is exactly the range a source can serve again — below
    /// it, [`crate::ingest::IngestError::FinalizedHistoryChanged`] refuses the
    /// transaction outright, so it can never be re-folded. Restoring that set is
    /// not an optimisation: `Staked`, `Unstaked`, `RewardsClaimed` and
    /// `StreamClaimed` accumulate, so without it the first redelivery after a
    /// restart double-counts.
    pub fn load(&mut self) -> Result<Restored, StoreError> {
        // One read-only transaction, so the cursor and the rows cannot come from
        // either side of a concurrent `commit`.
        let mut tx = self
            .client
            .transaction()
            .map_err(|source| StoreError::Query {
                what: "beginning the load transaction",
                source,
            })?;

        let cursor = read_cursor(&mut tx)?;
        let mut state = Analytics::new();
        read_realms(&mut tx, &mut state)?;
        read_pools(&mut tx, &mut state)?;
        read_positions(&mut tx, &mut state)?;
        read_proposals(&mut tx, &mut state)?;
        read_votes(&mut tx, &mut state)?;
        read_treasuries(&mut tx, &mut state)?;
        read_streams(&mut tx, &mut state)?;
        read_applied(&mut tx, &mut state, cursor.slot)?;
        read_orphans(&mut tx, &mut state)?;

        tx.commit().map_err(|source| StoreError::Query {
            what: "closing the load transaction",
            source,
        })?;

        Ok(Restored { cursor, state })
    }
}

// ------------------------------------------------------------------ what changed

/// Which rows a batch of events needs rewritten, and as of which slot.
///
/// Derived from the events rather than by writing the whole projection every
/// poll, which would make each commit cost the size of the protocol's history.
///
/// The slot is the highest at which this batch touched the entity, so
/// `updated_at_slot` keeps meaning "when this last changed" — which is what the
/// upsert guard compares.
#[derive(Default)]
struct Touched {
    realms: BTreeMap<Pubkey, u64>,
    pools: BTreeMap<Pubkey, u64>,
    positions: BTreeMap<Pubkey, u64>,
    proposals: BTreeMap<Pubkey, u64>,
    treasuries: BTreeMap<Pubkey, u64>,
    /// Keyed by the stream account; the value carries its treasury, because the
    /// projection nests streams under treasuries and the table does not.
    streams: BTreeMap<Pubkey, (Pubkey, u64)>,
    /// Votes are not in the projection at all — `ProposalStats` keeps the tally
    /// and the set of voters, deliberately, because the per-vote detail is this
    /// table. So these come straight off the events.
    votes: Vec<VoteRow>,
}

struct VoteRow {
    proposal: Pubkey,
    position: Pubkey,
    voter: Pubkey,
    choice: String,
    weight: u64,
    voted_at: i64,
}

impl Touched {
    fn of(settled: &[SettledTransaction]) -> Self {
        let mut touched = Self::default();
        for tx in settled {
            for emitted in &tx.events {
                touched.record(&emitted.event, tx.slot);
            }
        }
        touched
    }

    /// Exhaustive by construction: a new event type is a compile error here as
    /// well as in the fold, so "which rows does this change" cannot be forgotten
    /// the way a catch-all arm would let it be.
    fn record(&mut self, event: &HelixEvent, slot: u64) {
        use HelixEvent as E;
        let at = |map: &mut BTreeMap<Pubkey, u64>, key: Pubkey| {
            let entry = map.entry(key).or_insert(slot);
            *entry = (*entry).max(slot);
        };
        match event {
            // ------------------------------------------------------- staking
            E::PoolInitialized(e) => at(&mut self.pools, e.pool),
            E::RewardsFunded(e) => at(&mut self.pools, e.pool),
            E::RewardRateChanged(e) => at(&mut self.pools, e.pool),
            E::PoolPauseToggled(e) => at(&mut self.pools, e.pool),
            E::AuthorityTransferAccepted(e) => at(&mut self.pools, e.pool),
            E::AuthorityTransferProposed(_) => {}
            E::Staked(e) => {
                at(&mut self.pools, e.pool);
                at(&mut self.positions, e.position);
            }
            E::Unstaked(e) => {
                at(&mut self.pools, e.pool);
                at(&mut self.positions, e.position);
            }
            // The row is written from the projection, and the projection has
            // dropped it — so this resolves to a DELETE. Recording it is what
            // makes the deletion happen at all.
            E::PositionClosed(e) => {
                at(&mut self.pools, e.pool);
                at(&mut self.positions, e.position);
            }
            E::RewardsClaimed(e) => {
                at(&mut self.pools, e.pool);
                at(&mut self.positions, e.position);
            }

            // ---------------------------------------------------- governance
            E::RealmInitialized(e) => at(&mut self.realms, e.realm),
            E::RealmParamsUpdated(e) => at(&mut self.realms, e.realm),
            E::RealmAuthorityChanged(e) => at(&mut self.realms, e.realm),
            E::ProposalCreated(e) => at(&mut self.proposals, e.proposal),
            E::ProposalActivated(e) => at(&mut self.proposals, e.proposal),
            E::ProposalFinalized(e) => at(&mut self.proposals, e.proposal),
            E::ProposalQueued(e) => at(&mut self.proposals, e.proposal),
            E::ProposalExecuted(e) => at(&mut self.proposals, e.proposal),
            E::ProposalCancelled(e) => at(&mut self.proposals, e.proposal),
            E::VoteCast(e) => {
                at(&mut self.proposals, e.proposal);
                self.votes.push(VoteRow {
                    proposal: e.proposal,
                    position: e.position,
                    voter: e.voter,
                    choice: format!("{:?}", e.choice),
                    weight: e.weight,
                    voted_at: e.timestamp,
                });
            }

            // ------------------------------------------------------ treasury
            E::TreasuryInitialized(e) => at(&mut self.treasuries, e.treasury),
            E::Deposited(e) => at(&mut self.treasuries, e.treasury),
            E::Spent(e) => at(&mut self.treasuries, e.treasury),
            E::SpendCapChanged(e) => at(&mut self.treasuries, e.treasury),
            E::GovernanceExecutorChanged(e) => at(&mut self.treasuries, e.treasury),
            E::StreamCreated(e) => {
                at(&mut self.treasuries, e.treasury);
                self.streams.insert(e.stream, (e.treasury, slot));
            }
            E::StreamClaimed(e) => {
                at(&mut self.treasuries, e.treasury);
                self.streams.insert(e.stream, (e.treasury, slot));
            }
            E::StreamRevoked(e) => {
                at(&mut self.treasuries, e.treasury);
                self.streams.insert(e.stream, (e.treasury, slot));
            }

            // -------------------------------------------------- token-manager
            // Issuance is the token-manager's own business and has no row here,
            // matching `projection::Analytics::fold`. Listed rather than caught,
            // so adding an event forces a decision in both places.
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

// ----------------------------------------------------------------- writing

/// A u64 on its way into a `NUMERIC(20, 0)`. See the module header.
fn amount(value: u64) -> String {
    value.to_string()
}

fn key(value: &Pubkey) -> String {
    value.to_string()
}

fn run(
    tx: &mut Transaction<'_>,
    what: &'static str,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<u64, StoreError> {
    tx.execute(sql, params)
        .map_err(|source| StoreError::Query { what, source })
}

fn write_events(
    tx: &mut Transaction<'_>,
    settled: &[SettledTransaction],
) -> Result<usize, StoreError> {
    let mut inserted = 0usize;
    for settled_tx in settled {
        for emitted in &settled_tx.events {
            // `DO NOTHING`, not `DO UPDATE`. These rows are immutable history:
            // if the same (signature, log_index) ever carried different bytes,
            // overwriting would destroy the evidence of whichever party is
            // wrong, and the row already stored is the one that was verified
            // against the projection at the time.
            let affected = run(
                tx,
                "inserting an event",
                "INSERT INTO events \
                     (signature, log_index, slot, depth, program, kind, block_time, payload) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
                 ON CONFLICT (signature, log_index) DO NOTHING",
                &[
                    &settled_tx.signature,
                    &(emitted.log_index as i32),
                    &(settled_tx.slot as i64),
                    &(emitted.depth as i32),
                    &emitted.program.name(),
                    &emitted.event.name(),
                    &emitted.event.timestamp(),
                    &wire_bytes(&emitted.event),
                ],
            )?;
            inserted += affected as usize;
        }
    }
    Ok(inserted)
}

/// The bytes the chain wrote on the `Program data:` line.
///
/// Re-serialised from the decoded event rather than carried through from the log,
/// which is a deliberate round trip: it means a payload that would not decode back
/// to the row beside it cannot be stored, and it keeps `SettledTransaction` free
/// of a second copy of every log line.
fn wire_bytes(event: &HelixEvent) -> Vec<u8> {
    use anchor_lang::{AnchorSerialize, Discriminator};

    fn encode<T: Discriminator + AnchorSerialize>(event: &T) -> Vec<u8> {
        let mut bytes = T::DISCRIMINATOR.to_vec();
        // Serialising into a Vec cannot fail; Borsh only errors when the writer
        // does, and a Vec does not.
        event.serialize(&mut bytes).expect("in-memory write");
        bytes
    }

    macro_rules! encode_variants {
        ($($variant:ident),* $(,)?) => {
            match event {
                $( HelixEvent::$variant(e) => encode(e), )*
            }
        };
    }

    encode_variants!(
        TokenInitialized,
        MinterRegistered,
        MinterUpdated,
        MinterRevoked,
        TokensMinted,
        TokensBurned,
        AdminTransferProposed,
        AdminTransferCancelled,
        AdminTransferAccepted,
        PauseToggled,
        PoolInitialized,
        Staked,
        Unstaked,
        PositionClosed,
        RewardsClaimed,
        RewardsFunded,
        RewardRateChanged,
        PoolPauseToggled,
        AuthorityTransferProposed,
        AuthorityTransferAccepted,
        RealmInitialized,
        RealmParamsUpdated,
        RealmAuthorityChanged,
        ProposalCreated,
        ProposalActivated,
        VoteCast,
        ProposalFinalized,
        ProposalQueued,
        ProposalExecuted,
        ProposalCancelled,
        TreasuryInitialized,
        Deposited,
        Spent,
        StreamCreated,
        StreamClaimed,
        StreamRevoked,
        SpendCapChanged,
        GovernanceExecutorChanged,
    )
}

fn write_anomalies(
    tx: &mut Transaction<'_>,
    settled: &[SettledTransaction],
    state: &Analytics,
) -> Result<(), StoreError> {
    for settled_tx in settled {
        for anomaly in &settled_tx.anomalies {
            let (log_index, kind, detail) = match anomaly {
                Anomaly::Truncated { log_index } => (*log_index, "truncated", None),
                Anomaly::UndecodableData { log_index, program } => (
                    *log_index,
                    "undecodable",
                    Some(format!("emitted by {}", program.name())),
                ),
                Anomaly::UnbalancedInvokeStack { log_index } => (*log_index, "unbalanced", None),
            };
            insert_anomaly(tx, &settled_tx.signature, log_index, kind, detail)?;
        }
    }

    // Orphans are found by the fold rather than by the parser, so they are read
    // off the projection and filtered down to this batch — `state.orphaned`
    // accumulates for the life of the process, and rewriting all of it on every
    // commit would make each poll cost the run's whole history of gaps.
    //
    // An event that referenced an entity nobody saw created is expected on a
    // backfill and means something was dropped on a live stream; either way it
    // belongs with the other reasons the numbers might be incomplete.
    let in_batch: BTreeSet<&str> = settled.iter().map(|tx| tx.signature.as_str()).collect();
    for id in state.orphaned.iter() {
        if in_batch.contains(id.signature.as_str()) {
            insert_anomaly(tx, &id.signature, id.log_index, "orphaned", None)?;
        }
    }
    Ok(())
}

fn insert_anomaly(
    tx: &mut Transaction<'_>,
    signature: &str,
    log_index: usize,
    kind: &str,
    detail: Option<String>,
) -> Result<(), StoreError> {
    run(
        tx,
        "recording an ingestion anomaly",
        "INSERT INTO ingestion_anomalies (signature, log_index, kind, detail) \
         VALUES ($1, $2, $3, $4) \
         ON CONFLICT (signature, log_index, kind) DO NOTHING",
        &[&signature, &(log_index as i32), &kind, &detail],
    )?;
    Ok(())
}

fn write_realms(
    tx: &mut Transaction<'_>,
    state: &Analytics,
    touched: &Touched,
) -> Result<(), StoreError> {
    for (address, slot) in &touched.realms {
        let Some(realm) = state.realms.get(address) else {
            continue;
        };
        run(
            tx,
            "upserting a realm",
            "INSERT INTO realms (address, authority, guardian, staking_pool, quorum_bps, \
                 approval_bps, voting_period, timelock_delay, min_weight_to_propose, \
                 self_governing, partial_history, updated_at_slot) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9::text::numeric, $10, $11, $12) \
             ON CONFLICT (address) DO UPDATE SET \
                 authority = EXCLUDED.authority, \
                 guardian = EXCLUDED.guardian, \
                 staking_pool = EXCLUDED.staking_pool, \
                 quorum_bps = EXCLUDED.quorum_bps, \
                 approval_bps = EXCLUDED.approval_bps, \
                 voting_period = EXCLUDED.voting_period, \
                 timelock_delay = EXCLUDED.timelock_delay, \
                 min_weight_to_propose = EXCLUDED.min_weight_to_propose, \
                 self_governing = EXCLUDED.self_governing, \
                 partial_history = EXCLUDED.partial_history, \
                 updated_at_slot = EXCLUDED.updated_at_slot \
             WHERE realms.updated_at_slot <= EXCLUDED.updated_at_slot",
            &[
                &key(address),
                &realm.authority.as_ref().map(key),
                &realm.guardian.as_ref().map(key),
                &realm.staking_pool.as_ref().map(key),
                &(realm.quorum_bps as i32),
                &(realm.approval_bps as i32),
                &realm.voting_period,
                &realm.timelock_delay,
                &amount(realm.min_weight_to_propose),
                &realm.self_governing,
                &realm.partial_history,
                &(*slot as i64),
            ],
        )?;
    }
    Ok(())
}

fn write_pools(
    tx: &mut Transaction<'_>,
    state: &Analytics,
    touched: &Touched,
) -> Result<(), StoreError> {
    for (address, slot) in &touched.pools {
        let Some(pool) = state.pools.get(address) else {
            continue;
        };
        run(
            tx,
            "upserting a pool",
            "INSERT INTO pools (address, authority, total_staked, total_weighted, position_count, \
                 reward_rate, reward_period_end, total_rewards_funded, total_rewards_paid, \
                 paused, partial_history, updated_at_slot) \
             VALUES ($1, $2, $3::text::numeric, $4::text::numeric, $5, $6::text::numeric, $7, \
                 $8::text::numeric, $9::text::numeric, $10, $11, $12) \
             ON CONFLICT (address) DO UPDATE SET \
                 authority = EXCLUDED.authority, \
                 total_staked = EXCLUDED.total_staked, \
                 total_weighted = EXCLUDED.total_weighted, \
                 position_count = EXCLUDED.position_count, \
                 reward_rate = EXCLUDED.reward_rate, \
                 reward_period_end = EXCLUDED.reward_period_end, \
                 total_rewards_funded = EXCLUDED.total_rewards_funded, \
                 total_rewards_paid = EXCLUDED.total_rewards_paid, \
                 paused = EXCLUDED.paused, \
                 partial_history = EXCLUDED.partial_history, \
                 updated_at_slot = EXCLUDED.updated_at_slot \
             WHERE pools.updated_at_slot <= EXCLUDED.updated_at_slot",
            &[
                &key(address),
                &pool.authority.as_ref().map(key),
                &amount(pool.total_staked),
                &amount(pool.total_weighted),
                &(pool.position_count as i64),
                &amount(pool.reward_rate),
                &pool.reward_period_end,
                &amount(pool.total_rewards_funded),
                &amount(pool.total_rewards_paid),
                &pool.paused,
                &pool.partial_history,
                &(*slot as i64),
            ],
        )?;
    }
    Ok(())
}

fn write_positions(
    tx: &mut Transaction<'_>,
    state: &Analytics,
    touched: &Touched,
) -> Result<(), StoreError> {
    for (address, slot) in &touched.positions {
        let Some(position) = state.positions.get(address) else {
            // Absent from the projection means `close_position` deallocated the
            // account. Deleting rather than flagging, because the projection is
            // compared field by field against accounts that exist — see
            // `indexer_reconciliation.rs` — and a row for a closed account would
            // make the store disagree with the thing it mirrors.
            run(
                tx,
                "deleting a closed position",
                "DELETE FROM positions WHERE address = $1",
                &[&key(address)],
            )?;
            continue;
        };
        run(
            tx,
            "upserting a position",
            "INSERT INTO positions (address, pool, owner, position_id, amount, weighted_amount, \
                 tier, lock_end, rewards_claimed, updated_at_slot) \
             VALUES ($1, $2, $3, $4, $5::text::numeric, $6::text::numeric, $7, $8, \
                 $9::text::numeric, $10) \
             ON CONFLICT (address) DO UPDATE SET \
                 amount = EXCLUDED.amount, \
                 weighted_amount = EXCLUDED.weighted_amount, \
                 tier = EXCLUDED.tier, \
                 lock_end = EXCLUDED.lock_end, \
                 rewards_claimed = EXCLUDED.rewards_claimed, \
                 updated_at_slot = EXCLUDED.updated_at_slot \
             WHERE positions.updated_at_slot <= EXCLUDED.updated_at_slot",
            &[
                &key(address),
                &key(&position.pool),
                &key(&position.owner),
                &(position.position_id as i64),
                &amount(position.amount),
                &amount(position.weighted_amount),
                &format!("{:?}", position.tier),
                &position.lock_end,
                &amount(position.rewards_claimed),
                &(*slot as i64),
            ],
        )?;
    }
    Ok(())
}

fn write_proposals(
    tx: &mut Transaction<'_>,
    state: &Analytics,
    touched: &Touched,
) -> Result<(), StoreError> {
    for (address, slot) in &touched.proposals {
        let Some(proposal) = state.proposals.get(address) else {
            continue;
        };
        run(
            tx,
            "upserting a proposal",
            "INSERT INTO proposals (address, realm, proposal_id, proposer, title, state, \
                 for_votes, against_votes, abstain_votes, total_weight_snapshot, \
                 position_count_snapshot, eta, updated_at_slot) \
             VALUES ($1, $2, $3, $4, $5, $6, $7::text::numeric, $8::text::numeric, \
                 $9::text::numeric, $10::text::numeric, $11, $12, $13) \
             ON CONFLICT (address) DO UPDATE SET \
                 state = EXCLUDED.state, \
                 title = EXCLUDED.title, \
                 for_votes = EXCLUDED.for_votes, \
                 against_votes = EXCLUDED.against_votes, \
                 abstain_votes = EXCLUDED.abstain_votes, \
                 total_weight_snapshot = EXCLUDED.total_weight_snapshot, \
                 position_count_snapshot = EXCLUDED.position_count_snapshot, \
                 eta = EXCLUDED.eta, \
                 updated_at_slot = EXCLUDED.updated_at_slot \
             WHERE proposals.updated_at_slot <= EXCLUDED.updated_at_slot",
            &[
                &key(address),
                &key(&proposal.realm),
                &(proposal.id as i64),
                &key(&proposal.proposer),
                &proposal.title,
                &format!("{:?}", proposal.state),
                &amount(proposal.for_votes),
                &amount(proposal.against_votes),
                &amount(proposal.abstain_votes),
                &amount(proposal.total_weight_snapshot),
                &(proposal.position_count_snapshot as i64),
                &proposal.eta,
                &(*slot as i64),
            ],
        )?;
    }
    Ok(())
}

fn write_votes(tx: &mut Transaction<'_>, touched: &Touched) -> Result<(), StoreError> {
    for vote in &touched.votes {
        // `DO NOTHING`: the on-chain `VoteRecord` is created with `init`, so a
        // second vote from one position is impossible and a second *row* could
        // only come from redelivery. Overwriting would be equally correct today
        // and would silently start absorbing a real double-vote if the program
        // ever stopped preventing one.
        run(
            tx,
            "recording a vote",
            "INSERT INTO votes (proposal, position, voter, choice, weight, voted_at) \
             VALUES ($1, $2, $3, $4, $5::text::numeric, $6) \
             ON CONFLICT (proposal, position) DO NOTHING",
            &[
                &key(&vote.proposal),
                &key(&vote.position),
                &key(&vote.voter),
                &vote.choice,
                &amount(vote.weight),
                &vote.voted_at,
            ],
        )?;
    }
    Ok(())
}

fn write_treasuries(
    tx: &mut Transaction<'_>,
    state: &Analytics,
    touched: &Touched,
) -> Result<(), StoreError> {
    for (address, slot) in &touched.treasuries {
        let Some(treasury) = state.treasuries.get(address) else {
            continue;
        };
        run(
            tx,
            "upserting a treasury",
            "INSERT INTO treasuries (address, governance_executor, total_deposited, total_spent, \
                 total_stream_claims, epoch_spend_cap, partial_history, updated_at_slot) \
             VALUES ($1, $2, $3::text::numeric, $4::text::numeric, $5::text::numeric, \
                 $6::text::numeric, $7, $8) \
             ON CONFLICT (address) DO UPDATE SET \
                 governance_executor = EXCLUDED.governance_executor, \
                 total_deposited = EXCLUDED.total_deposited, \
                 total_spent = EXCLUDED.total_spent, \
                 total_stream_claims = EXCLUDED.total_stream_claims, \
                 epoch_spend_cap = EXCLUDED.epoch_spend_cap, \
                 partial_history = EXCLUDED.partial_history, \
                 updated_at_slot = EXCLUDED.updated_at_slot \
             WHERE treasuries.updated_at_slot <= EXCLUDED.updated_at_slot",
            &[
                &key(address),
                &treasury.governance_executor.as_ref().map(key),
                &amount(treasury.total_deposited),
                &amount(treasury.total_spent),
                &amount(treasury.total_stream_claims),
                &amount(treasury.epoch_spend_cap),
                &treasury.partial_history,
                &(*slot as i64),
            ],
        )?;
    }
    Ok(())
}

fn write_streams(
    tx: &mut Transaction<'_>,
    state: &Analytics,
    touched: &Touched,
) -> Result<(), StoreError> {
    for (address, (treasury_key, slot)) in &touched.streams {
        let Some(stream) = state
            .treasuries
            .get(treasury_key)
            .and_then(|t| t.open_streams.get(address))
        else {
            continue;
        };
        run(
            tx,
            "upserting a stream",
            "INSERT INTO streams (address, treasury, stream_id, beneficiary, total_amount, \
                 claimed, revoked, updated_at_slot) \
             VALUES ($1, $2, $3, $4, $5::text::numeric, $6::text::numeric, $7, $8) \
             ON CONFLICT (address) DO UPDATE SET \
                 total_amount = EXCLUDED.total_amount, \
                 claimed = EXCLUDED.claimed, \
                 revoked = EXCLUDED.revoked, \
                 updated_at_slot = EXCLUDED.updated_at_slot \
             WHERE streams.updated_at_slot <= EXCLUDED.updated_at_slot",
            &[
                &key(address),
                &key(treasury_key),
                &(stream.stream_id as i64),
                &key(&stream.beneficiary),
                &amount(stream.total_amount),
                &amount(stream.claimed),
                &stream.revoked,
                &(*slot as i64),
            ],
        )?;
    }
    Ok(())
}

fn write_cursor(tx: &mut Transaction<'_>, cursor: &Cursor) -> Result<(), StoreError> {
    // No slot guard here, unlike the projection rows. The cursor is this process's
    // own position, not a fact about the chain that two writers could race on, and
    // guarding it would silently ignore a legitimate restart from an earlier point.
    run(
        tx,
        "advancing the cursor",
        "INSERT INTO cursors (name, slot, signature, updated_at) VALUES ($1, $2, $3, now()) \
         ON CONFLICT (name) DO UPDATE SET \
             slot = EXCLUDED.slot, signature = EXCLUDED.signature, updated_at = now()",
        &[&LIVE_CURSOR, &(cursor.slot as i64), &cursor.signature],
    )?;
    Ok(())
}

// ----------------------------------------------------------------- reading

fn parse_key(table: &'static str, column: &'static str, raw: &str) -> Result<Pubkey, StoreError> {
    Pubkey::from_str(raw).map_err(|_| StoreError::Malformed {
        table,
        column,
        value: raw.to_owned(),
    })
}

fn parse_amount(table: &'static str, column: &'static str, raw: &str) -> Result<u64, StoreError> {
    raw.parse().map_err(|_| StoreError::Malformed {
        table,
        column,
        value: raw.to_owned(),
    })
}

fn query(
    tx: &mut Transaction<'_>,
    what: &'static str,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<Vec<postgres::Row>, StoreError> {
    tx.query(sql, params)
        .map_err(|source| StoreError::Query { what, source })
}

fn read_cursor(tx: &mut Transaction<'_>) -> Result<Cursor, StoreError> {
    let rows = query(
        tx,
        "reading the cursor",
        "SELECT slot, signature FROM cursors WHERE name = $1",
        &[&LIVE_CURSOR],
    )?;
    // A database that has never been written to yields the default cursor, which
    // is genesis — the correct answer, and the one that makes a first run and a
    // resumed run the same code path.
    Ok(rows.first().map_or_else(Cursor::default, |row| Cursor {
        slot: row.get::<_, i64>(0) as u64,
        signature: row.get(1),
    }))
}

fn read_realms(tx: &mut Transaction<'_>, state: &mut Analytics) -> Result<(), StoreError> {
    for row in query(
        tx,
        "reading realms",
        "SELECT address, authority, guardian, staking_pool, quorum_bps, approval_bps, \
             voting_period, timelock_delay, min_weight_to_propose::text, self_governing, \
             partial_history \
         FROM realms",
        &[],
    )? {
        let address = parse_key("realms", "address", row.get(0))?;
        let optional = |raw: Option<&str>, column| match raw {
            Some(raw) => parse_key("realms", column, raw).map(Some),
            None => Ok(None),
        };
        state.realms.insert(
            address,
            RealmStats {
                authority: optional(row.get(1), "authority")?,
                guardian: optional(row.get(2), "guardian")?,
                staking_pool: optional(row.get(3), "staking_pool")?,
                quorum_bps: row.get::<_, i32>(4) as u16,
                approval_bps: row.get::<_, i32>(5) as u16,
                voting_period: row.get(6),
                timelock_delay: row.get(7),
                min_weight_to_propose: parse_amount("realms", "min_weight_to_propose", row.get(8))?,
                self_governing: row.get(9),
                partial_history: row.get(10),
            },
        );
    }
    Ok(())
}

fn read_pools(tx: &mut Transaction<'_>, state: &mut Analytics) -> Result<(), StoreError> {
    for row in query(
        tx,
        "reading pools",
        "SELECT address, authority, total_staked::text, total_weighted::text, position_count, \
             reward_rate::text, reward_period_end, total_rewards_funded::text, \
             total_rewards_paid::text, paused, partial_history \
         FROM pools",
        &[],
    )? {
        let address = parse_key("pools", "address", row.get(0))?;
        let authority = match row.get::<_, Option<&str>>(1) {
            Some(raw) => Some(parse_key("pools", "authority", raw)?),
            None => None,
        };
        state.pools.insert(
            address,
            PoolStats {
                authority,
                total_staked: parse_amount("pools", "total_staked", row.get(2))?,
                total_weighted: parse_amount("pools", "total_weighted", row.get(3))?,
                position_count: row.get::<_, i64>(4) as u64,
                reward_rate: parse_amount("pools", "reward_rate", row.get(5))?,
                reward_period_end: row.get(6),
                total_rewards_funded: parse_amount("pools", "total_rewards_funded", row.get(7))?,
                total_rewards_paid: parse_amount("pools", "total_rewards_paid", row.get(8))?,
                paused: row.get(9),
                partial_history: row.get(10),
            },
        );
    }
    Ok(())
}

fn read_positions(tx: &mut Transaction<'_>, state: &mut Analytics) -> Result<(), StoreError> {
    for row in query(
        tx,
        "reading positions",
        "SELECT address, pool, owner, position_id, amount::text, weighted_amount::text, tier, \
             lock_end, rewards_claimed::text \
         FROM positions",
        &[],
    )? {
        let address = parse_key("positions", "address", row.get(0))?;
        let tier_name: &str = row.get(6);
        state.positions.insert(
            address,
            PositionStats {
                pool: parse_key("positions", "pool", row.get(1))?,
                owner: parse_key("positions", "owner", row.get(2))?,
                position_id: row.get::<_, i64>(3) as u64,
                amount: parse_amount("positions", "amount", row.get(4))?,
                weighted_amount: parse_amount("positions", "weighted_amount", row.get(5))?,
                tier: parse_tier(tier_name)?,
                lock_end: row.get(7),
                rewards_claimed: parse_amount("positions", "rewards_claimed", row.get(8))?,
            },
        );
    }
    Ok(())
}

/// The lock tier, back from the name the writer stored.
///
/// Written as `{:?}` and read by matching the same names, which is a round trip
/// through a `Debug` impl and therefore something a rename would break silently
/// in one direction. `store_roundtrip.rs` covers every variant for exactly that
/// reason — the failure is a position that loads at the wrong weight, which is
/// vote weight, which is a treasury.
fn parse_tier(name: &str) -> Result<helix_staking::state::LockTier, StoreError> {
    use helix_staking::state::LockTier;
    let tier = match name {
        "Flexible" => LockTier::Flexible,
        "Bronze" => LockTier::Bronze,
        "Silver" => LockTier::Silver,
        "Gold" => LockTier::Gold,
        _ => {
            return Err(StoreError::Malformed {
                table: "positions",
                column: "tier",
                value: name.to_owned(),
            })
        }
    };
    Ok(tier)
}

fn read_proposals(tx: &mut Transaction<'_>, state: &mut Analytics) -> Result<(), StoreError> {
    for row in query(
        tx,
        "reading proposals",
        "SELECT address, realm, proposal_id, proposer, title, state, for_votes::text, \
             against_votes::text, abstain_votes::text, total_weight_snapshot::text, \
             position_count_snapshot, eta \
         FROM proposals",
        &[],
    )? {
        let address = parse_key("proposals", "address", row.get(0))?;
        let state_name: &str = row.get(5);
        state.proposals.insert(
            address,
            ProposalStats {
                realm: parse_key("proposals", "realm", row.get(1))?,
                id: row.get::<_, i64>(2) as u64,
                proposer: parse_key("proposals", "proposer", row.get(3))?,
                title: row.get(4),
                state: parse_proposal_state(state_name)?,
                for_votes: parse_amount("proposals", "for_votes", row.get(6))?,
                against_votes: parse_amount("proposals", "against_votes", row.get(7))?,
                abstain_votes: parse_amount("proposals", "abstain_votes", row.get(8))?,
                total_weight_snapshot: parse_amount(
                    "proposals",
                    "total_weight_snapshot",
                    row.get(9),
                )?,
                position_count_snapshot: row.get::<_, i64>(10) as u64,
                // Filled in by `read_votes`, which is why it runs after this.
                voters: BTreeSet::new(),
                eta: row.get(11),
            },
        );
    }
    Ok(())
}

fn parse_proposal_state(name: &str) -> Result<helix_governance::state::ProposalState, StoreError> {
    use helix_governance::state::ProposalState as S;
    let state = match name {
        "Draft" => S::Draft,
        "Voting" => S::Voting,
        "Succeeded" => S::Succeeded,
        "Defeated" => S::Defeated,
        "Queued" => S::Queued,
        "Executed" => S::Executed,
        "Cancelled" => S::Cancelled,
        _ => {
            return Err(StoreError::Malformed {
                table: "proposals",
                column: "state",
                value: name.to_owned(),
            })
        }
    };
    Ok(state)
}

/// Rebuilds each proposal's voter set from the vote rows.
///
/// The projection keeps only the set; the per-vote detail lives in the table. So
/// the set is derived from the rows rather than stored twice, which means the two
/// cannot disagree.
fn read_votes(tx: &mut Transaction<'_>, state: &mut Analytics) -> Result<(), StoreError> {
    for row in query(
        tx,
        "reading votes",
        "SELECT proposal, voter FROM votes",
        &[],
    )? {
        let proposal = parse_key("votes", "proposal", row.get(0))?;
        let voter = parse_key("votes", "voter", row.get(1))?;
        if let Some(proposal) = state.proposals.get_mut(&proposal) {
            proposal.voters.insert(voter);
        }
    }
    Ok(())
}

fn read_treasuries(tx: &mut Transaction<'_>, state: &mut Analytics) -> Result<(), StoreError> {
    for row in query(
        tx,
        "reading treasuries",
        "SELECT address, governance_executor, total_deposited::text, total_spent::text, \
             total_stream_claims::text, epoch_spend_cap::text, partial_history \
         FROM treasuries",
        &[],
    )? {
        let address = parse_key("treasuries", "address", row.get(0))?;
        let executor = match row.get::<_, Option<&str>>(1) {
            Some(raw) => Some(parse_key("treasuries", "governance_executor", raw)?),
            None => None,
        };
        state.treasuries.insert(
            address,
            TreasuryStats {
                governance_executor: executor,
                total_deposited: parse_amount("treasuries", "total_deposited", row.get(2))?,
                total_spent: parse_amount("treasuries", "total_spent", row.get(3))?,
                total_stream_claims: parse_amount("treasuries", "total_stream_claims", row.get(4))?,
                epoch_spend_cap: parse_amount("treasuries", "epoch_spend_cap", row.get(5))?,
                partial_history: row.get(6),
                open_streams: BTreeMap::new(),
            },
        );
    }
    Ok(())
}

fn read_streams(tx: &mut Transaction<'_>, state: &mut Analytics) -> Result<(), StoreError> {
    for row in query(
        tx,
        "reading streams",
        "SELECT address, treasury, stream_id, beneficiary, total_amount::text, claimed::text, \
             revoked \
         FROM streams",
        &[],
    )? {
        let address = parse_key("streams", "address", row.get(0))?;
        let treasury = parse_key("streams", "treasury", row.get(1))?;
        let stats = StreamStats {
            stream_id: row.get::<_, i64>(2) as u64,
            beneficiary: parse_key("streams", "beneficiary", row.get(3))?,
            total_amount: parse_amount("streams", "total_amount", row.get(4))?,
            claimed: parse_amount("streams", "claimed", row.get(5))?,
            revoked: row.get(6),
        };
        // The foreign key guarantees the treasury row exists, and `read_treasuries`
        // runs first, so this cannot silently drop a stream.
        state
            .treasuries
            .entry(treasury)
            .or_default()
            .open_streams
            .insert(address, stats);
    }
    Ok(())
}

/// Restores the dedup set for everything a source could serve again.
///
/// `slot >= cursor.slot`, and that bound is exact rather than cautious. Below the
/// cursor, the ingestor refuses the transaction outright as
/// `FinalizedHistoryChanged` — finalised history does not change, so a source
/// offering it is either lying or serving a different ledger — which means those
/// events can never be re-folded and their ids need not be held. At the cursor's
/// own slot they can: the cursor resumes mid-slot by signature, and an RPC source
/// whose transaction-status index has pruned that signature re-serves the whole
/// slot. So the set is one slot deep, not the whole of history.
fn read_applied(
    tx: &mut Transaction<'_>,
    state: &mut Analytics,
    cursor_slot: u64,
) -> Result<(), StoreError> {
    for row in query(
        tx,
        "reading the applied set",
        "SELECT signature, log_index FROM events WHERE slot >= $1",
        &[&(cursor_slot as i64)],
    )? {
        state.mark_applied(EventId {
            signature: row.get(0),
            log_index: row.get::<_, i32>(1) as usize,
        });
    }
    Ok(())
}

/// Restores what the projection knows it is missing.
///
/// Without this a restarted indexer reports zero orphans while the database holds
/// hundreds, which turns "the stream started mid-history" from a recorded fact
/// into something that looks fixed.
fn read_orphans(tx: &mut Transaction<'_>, state: &mut Analytics) -> Result<(), StoreError> {
    for row in query(
        tx,
        "reading orphaned events",
        "SELECT signature, log_index FROM ingestion_anomalies WHERE kind = 'orphaned'",
        &[],
    )? {
        state.orphaned.insert(EventId {
            signature: row.get(0),
            log_index: row.get::<_, i32>(1) as usize,
        });
    }
    Ok(())
}

// -------------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Program;
    use crate::logs::EmittedEvent;
    use helix_staking::state::LockTier;

    fn settled(slot: u64, signature: &str, events: Vec<HelixEvent>) -> SettledTransaction {
        SettledTransaction {
            signature: signature.to_owned(),
            slot,
            events: events
                .into_iter()
                .enumerate()
                .map(|(log_index, event)| EmittedEvent {
                    program: event.program(),
                    event,
                    log_index,
                    depth: 1,
                })
                .collect(),
            anomalies: Vec::new(),
        }
    }

    fn staked(pool: Pubkey, position: Pubkey) -> HelixEvent {
        HelixEvent::Staked(helix_staking::events::Staked {
            pool,
            position,
            owner: Pubkey::new_unique(),
            position_id: 0,
            amount_sent: 1_000,
            amount_credited: 1_000,
            weighted_amount: 1_000,
            tier: LockTier::Flexible,
            lock_end: 0,
            timestamp: 1,
        })
    }

    /// The one thing that decides what a commit costs.
    #[test]
    fn only_the_entities_a_batch_touched_are_rewritten() {
        let (pool, position) = (Pubkey::new_unique(), Pubkey::new_unique());
        let touched = Touched::of(&[settled(7, "sig", vec![staked(pool, position)])]);

        assert_eq!(touched.pools.keys().collect::<Vec<_>>(), vec![&pool]);
        assert_eq!(
            touched.positions.keys().collect::<Vec<_>>(),
            vec![&position]
        );
        assert!(touched.realms.is_empty());
        assert!(touched.treasuries.is_empty());
        assert!(touched.proposals.is_empty());
    }

    /// `updated_at_slot` is the upsert guard, so it has to be the highest slot at
    /// which the batch touched the row. Taking the first would let a later commit
    /// carrying an older figure win.
    #[test]
    fn an_entity_touched_twice_records_the_later_slot() {
        let pool = Pubkey::new_unique();
        let touched = Touched::of(&[
            settled(10, "a", vec![staked(pool, Pubkey::new_unique())]),
            settled(20, "b", vec![staked(pool, Pubkey::new_unique())]),
        ]);
        assert_eq!(touched.pools[&pool], 20);
    }

    /// Every event must survive the round trip through the `payload` column, or
    /// the claim that the projection is replayable from storage is false.
    #[test]
    fn the_stored_payload_decodes_back_to_the_event() {
        let event = staked(Pubkey::new_unique(), Pubkey::new_unique());
        let bytes = wire_bytes(&event);
        assert_eq!(
            HelixEvent::decode(Program::Staking, &bytes),
            Some(event),
            "the bytes written to `events.payload` do not decode back"
        );
    }

    /// The two `Debug`-name round trips, pinned per variant.
    ///
    /// Both are written with `{:?}` and read by matching literals, so a rename
    /// breaks one side only. For the tier the consequence is a position loading at
    /// the wrong vote weight; for the state it is a proposal loading as `Draft`.
    #[test]
    fn every_tier_and_state_name_round_trips() {
        for tier in [
            LockTier::Flexible,
            LockTier::Bronze,
            LockTier::Silver,
            LockTier::Gold,
        ] {
            assert_eq!(parse_tier(&format!("{tier:?}")).expect("tier"), tier);
        }

        use helix_governance::state::ProposalState as S;
        for state in [
            S::Draft,
            S::Voting,
            S::Succeeded,
            S::Defeated,
            S::Queued,
            S::Executed,
            S::Cancelled,
        ] {
            assert_eq!(
                parse_proposal_state(&format!("{state:?}")).expect("state"),
                state
            );
        }
    }

    #[test]
    fn an_unknown_tier_name_is_an_error_rather_than_a_default() {
        assert!(matches!(
            parse_tier("Platinum"),
            Err(StoreError::Malformed { column: "tier", .. })
        ));
    }

    /// u64 has to cross the boundary as text, because `BIGINT` is signed.
    #[test]
    fn an_amount_above_the_signed_range_survives_as_text() {
        let huge = u64::MAX;
        assert!(huge > i64::MAX as u64);
        assert_eq!(
            parse_amount("pools", "total_staked", &amount(huge)).expect("round trip"),
            huge
        );
    }
}
