//! The storage binding, against a real Postgres.
//!
//! An `ON CONFLICT` clause nobody has executed is a claim, not a guarantee, and
//! the specific claims here are the ones that fail quietly rather than loudly:
//! a cursor that advances past rows a crash discarded, an upsert a backfill
//! overwrites with older figures, a projection that loads back subtly smaller
//! than the one that was saved. None of those produce an error. All of them
//! produce numbers.
//!
//! ```text
//! docker run -d --name helix-postgres -e POSTGRES_PASSWORD=helix \
//!     -e POSTGRES_USER=helix -e POSTGRES_DB=helix -p 55432:5432 postgres:16-alpine
//! HELIX_DATABASE_URL=postgres://helix:helix@127.0.0.1:55432/helix \
//!     cargo test -p helix-integration-tests --test store_postgres
//! ```
//!
//! Without `HELIX_DATABASE_URL` every test here prints why it is doing nothing
//! and returns green, the same way the live RPC tests do. That is deliberate: a
//! suite that fails on a clean checkout trains people to ignore red, and an
//! `#[ignore]` hides the reason behind a flag someone has to already know about.
//!
//! # Isolation
//!
//! Each test gets its own Postgres *schema* and a connection whose `search_path`
//! points at it, so the whole of `schema.sql` — which names no schema — lands
//! there. Cargo runs these in parallel; sharing one set of tables would make
//! every assertion about row counts depend on what else happened to be running.
//!
//! # What is deliberately not tested through the binding
//!
//! The assertions read rows with a second, plain `postgres::Client` rather than
//! through `Store::load`. A round trip that writes and reads with the same code
//! cannot tell a correct encoding from a symmetrically wrong one — store the tier
//! as `"Gold"` and read `"Gold"` back and the test passes whether or not the
//! column means anything. Where the round trip *is* the claim, it says so.

use helix_indexer::ingest::SettledTransaction;
use helix_indexer::projection::Analytics;
use helix_indexer::EmittedEvent;
use helix_indexer::{Cursor, HelixEvent, Ingestor, Store, StoreError};
use helix_staking::state::LockTier;

use anchor_lang::prelude::Pubkey;
use std::sync::atomic::{AtomicU32, Ordering};

const DATABASE_URL_ENV: &str = "HELIX_DATABASE_URL";

/// A private schema, and a store pointed at it.
struct Scratch {
    /// The schema name, so a failing run can be inspected before the drop.
    schema: String,
    base_url: String,
    store: Store,
}

impl Scratch {
    /// `None` when no database is configured.
    fn open(label: &str) -> Option<Self> {
        let base_url = std::env::var(DATABASE_URL_ENV).ok()?;

        // A counter as well as the label, so a test that wants two independent
        // schemas can have them.
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let schema = format!(
            "helix_test_{label}_{}",
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );

        let mut admin = postgres::Client::connect(&base_url, postgres::NoTls)
            .expect("connect to HELIX_DATABASE_URL");
        admin
            .batch_execute(&format!(
                "DROP SCHEMA IF EXISTS {schema} CASCADE; CREATE SCHEMA {schema};"
            ))
            .expect("create the scratch schema");

        let store = Store::connect(&scoped(&base_url, &schema)).expect("connect the store");
        let mut scratch = Scratch {
            schema,
            base_url,
            store,
        };
        scratch.store.migrate().expect("apply schema.sql");
        Some(scratch)
    }

    /// A second connection to the same schema, for reading rows the binding wrote
    /// without going back through the binding.
    fn raw(&self) -> postgres::Client {
        postgres::Client::connect(&scoped(&self.base_url, &self.schema), postgres::NoTls)
            .expect("second connection")
    }

    /// A second `Store` on the same schema — a restarted process.
    fn reopen(&self) -> Store {
        Store::connect(&scoped(&self.base_url, &self.schema)).expect("reconnect")
    }

    fn count(&self, table: &str) -> i64 {
        self.raw()
            .query_one(&format!("SELECT count(*) FROM {table}"), &[])
            .expect("count")
            .get(0)
    }
}

/// Appends a `search_path` to a libpq URL.
fn scoped(url: &str, schema: &str) -> String {
    let separator = if url.contains('?') { '&' } else { '?' };
    format!("{url}{separator}options=-c%20search_path%3D{schema}")
}

/// Returns from the calling test when no database is configured.
macro_rules! store_or_skip {
    ($label:expr) => {
        match Scratch::open($label) {
            Some(scratch) => scratch,
            None => {
                eprintln!(
                    "skipped: set {} to a Postgres URL to run this test",
                    DATABASE_URL_ENV
                );
                return;
            }
        }
    };
}

// ------------------------------------------------------------------- fixtures

fn emitted(events: Vec<HelixEvent>) -> Vec<EmittedEvent> {
    events
        .into_iter()
        .enumerate()
        .map(|(log_index, event)| EmittedEvent {
            program: event.program(),
            event,
            log_index,
            depth: 1,
        })
        .collect()
}

fn settled(slot: u64, signature: &str, events: Vec<HelixEvent>) -> SettledTransaction {
    SettledTransaction {
        signature: signature.to_owned(),
        slot,
        events: emitted(events),
        anomalies: Vec::new(),
    }
}

/// Folds a batch into a projection and writes it, the way the binary does.
fn apply_and_commit(
    store: &mut Store,
    state: &mut Analytics,
    batch: &[SettledTransaction],
) -> Result<usize, StoreError> {
    for tx in batch {
        state.apply_transaction(&tx.signature, &tx.events);
    }
    let last = batch.last().expect("a batch with no transactions");
    let cursor = Cursor {
        slot: last.slot,
        signature: Some(last.signature.clone()),
    };
    store.commit(&cursor, state, batch)
}

fn pool_initialized(pool: Pubkey, authority: Pubkey) -> HelixEvent {
    HelixEvent::PoolInitialized(helix_staking::events::PoolInitialized {
        pool,
        authority,
        stake_mint: Pubkey::new_unique(),
        reward_mint: Pubkey::new_unique(),
        timestamp: 1,
    })
}

fn staked(pool: Pubkey, position: Pubkey, owner: Pubkey, amount: u64) -> HelixEvent {
    HelixEvent::Staked(helix_staking::events::Staked {
        pool,
        position,
        owner,
        position_id: 0,
        amount_sent: amount,
        amount_credited: amount,
        weighted_amount: LockTier::Gold.apply_weight(amount).expect("weight"),
        tier: LockTier::Gold,
        lock_end: 5_000,
        timestamp: 10,
    })
}

// ------------------------------------------------------------------ the tests

/// The whole point: a process that stops comes back knowing what it knew.
///
/// Every map in the projection is populated, then reloaded into an empty one and
/// compared. Comparing the whole `Analytics` rather than a few fields is what
/// makes this survive someone adding a field — a new column nobody persists shows
/// up here rather than as a zero on a dashboard.
#[test]
fn a_restart_reaches_the_state_it_saved() {
    let mut scratch = store_or_skip!("restart");

    let (pool, realm, treasury) = (
        Pubkey::new_unique(),
        Pubkey::new_unique(),
        Pubkey::new_unique(),
    );
    let (position, proposal, stream) = (
        Pubkey::new_unique(),
        Pubkey::new_unique(),
        Pubkey::new_unique(),
    );
    let (owner, voter) = (Pubkey::new_unique(), Pubkey::new_unique());

    let mut state = Analytics::new();
    let batch = vec![
        settled(
            100,
            "sig-a",
            vec![
                pool_initialized(pool, Pubkey::new_unique()),
                HelixEvent::RealmInitialized(helix_governance::events::RealmInitialized {
                    realm,
                    authority: Pubkey::new_unique(),
                    guardian: Pubkey::new_unique(),
                    staking_pool: pool,
                    quorum_bps: 1_500,
                    approval_bps: 6_000,
                    voting_period: 259_200,
                    timelock_delay: 86_400,
                    min_weight_to_propose: 5_000,
                    timestamp: 5,
                }),
                HelixEvent::TreasuryInitialized(helix_treasury::events::TreasuryInitialized {
                    treasury,
                    governance_executor: Pubkey::new_unique(),
                    mint: Pubkey::new_unique(),
                    epoch_spend_cap: 1_000_000,
                    epoch_duration: 86_400,
                    timestamp: 5,
                }),
            ],
        ),
        settled(
            101,
            "sig-b",
            vec![
                staked(pool, position, owner, 7_000),
                HelixEvent::Deposited(helix_treasury::events::Deposited {
                    treasury,
                    depositor: Pubkey::new_unique(),
                    amount_credited: 900,
                    total_deposited: 900,
                    timestamp: 11,
                }),
                HelixEvent::StreamCreated(helix_treasury::events::StreamCreated {
                    treasury,
                    stream,
                    beneficiary: Pubkey::new_unique(),
                    stream_id: 0,
                    total_amount: 400,
                    start_ts: 0,
                    cliff_ts: 0,
                    end_ts: 100,
                    timestamp: 11,
                }),
            ],
        ),
        settled(
            102,
            "sig-c",
            vec![
                HelixEvent::ProposalCreated(helix_governance::events::ProposalCreated {
                    realm,
                    proposal,
                    proposer: owner,
                    id: 0,
                    action: helix_governance::state::ProposalAction::Signal,
                    title: "pause the pool".into(),
                    timestamp: 12,
                }),
                HelixEvent::ProposalActivated(helix_governance::events::ProposalActivated {
                    proposal,
                    voting_starts_at: 12,
                    voting_ends_at: 100,
                    total_weight_snapshot: 10_000,
                    position_count_snapshot: 1,
                    timestamp: 12,
                }),
                HelixEvent::VoteCast(helix_governance::events::VoteCast {
                    proposal,
                    position,
                    voter,
                    choice: helix_governance::state::VoteChoice::For,
                    weight: 7_000,
                    for_votes: 7_000,
                    against_votes: 0,
                    abstain_votes: 0,
                    timestamp: 13,
                }),
            ],
        ),
    ];

    let written = apply_and_commit(&mut scratch.store, &mut state, &batch).expect("commit");

    // If the fixture ever stopped producing rows this test would pass by
    // comparing two empty projections, which is the failure mode of every
    // "save and load" test ever written.
    assert!(written >= 9, "only {written} event rows were written");
    for (name, populated) in [
        ("pools", !state.pools.is_empty()),
        ("positions", !state.positions.is_empty()),
        ("proposals", !state.proposals.is_empty()),
        ("treasuries", !state.treasuries.is_empty()),
        ("realms", !state.realms.is_empty()),
    ] {
        assert!(
            populated,
            "the fixture produced no {name}, so this proves nothing"
        );
    }
    assert!(
        !state.proposals[&proposal].voters.is_empty(),
        "no vote was recorded, so the votes table is untested"
    );

    let restored = scratch.reopen().load().expect("load");

    assert_eq!(restored.cursor.slot, 102);
    assert_eq!(restored.cursor.signature.as_deref(), Some("sig-c"));
    assert_eq!(restored.state.pools, state.pools, "pools differ");
    assert_eq!(
        restored.state.positions, state.positions,
        "positions differ"
    );
    assert_eq!(
        restored.state.proposals, state.proposals,
        "proposals differ — the voter set is rebuilt from the votes table"
    );
    assert_eq!(
        restored.state.treasuries, state.treasuries,
        "treasuries differ — streams are nested here and flat in the schema"
    );
    assert_eq!(restored.state.realms, state.realms, "realms differ");
}

/// Redelivery is routine, so committing the same batch twice must be free.
#[test]
fn committing_the_same_batch_twice_changes_nothing() {
    let mut scratch = store_or_skip!("idempotent");

    let (pool, position) = (Pubkey::new_unique(), Pubkey::new_unique());
    let batch = vec![settled(
        50,
        "sig-a",
        vec![
            pool_initialized(pool, Pubkey::new_unique()),
            staked(pool, position, Pubkey::new_unique(), 4_000),
        ],
    )];

    let mut state = Analytics::new();
    let first = apply_and_commit(&mut scratch.store, &mut state, &batch).expect("first commit");
    assert_eq!(
        first, 2,
        "the first commit wrote nothing to be idempotent about"
    );

    let before = scratch.reopen().load().expect("load").state;

    // The same batch again — what an at-least-once source does after a
    // reconnection, and what a backfill overlapping a live stream does routinely.
    let second = apply_and_commit(&mut scratch.store, &mut state, &batch).expect("second commit");
    assert_eq!(second, 0, "the replay inserted event rows a second time");

    let after = scratch.reopen().load().expect("load");
    assert_eq!(after.state.pools, before.pools);
    assert_eq!(after.state.positions, before.positions);
    assert_eq!(scratch.count("events"), 2);
}

/// The hazard the applied set exists for.
///
/// Most of the projection assigns running totals the events carry, so replaying
/// one converges. `Staked` is not one of those: the chain publishes the deposit
/// and no cumulative figure, so the fold accumulates and idempotency comes
/// entirely from `(signature, log_index)` having been seen before. That memory is
/// in the process, and a restart is precisely the moment it is gone.
///
/// A source re-serving the cursor's own slot is not exotic — the cursor resumes
/// mid-slot by signature, and an RPC node that has pruned that signature from its
/// address index serves the whole slot again.
#[test]
fn a_redelivery_after_a_restart_does_not_double_count() {
    let mut scratch = store_or_skip!("redelivery");

    let (pool, position) = (Pubkey::new_unique(), Pubkey::new_unique());
    let batch = vec![settled(
        900,
        "sig-a",
        vec![
            pool_initialized(pool, Pubkey::new_unique()),
            staked(pool, position, Pubkey::new_unique(), 6_000),
        ],
    )];

    let mut live = Analytics::new();
    apply_and_commit(&mut scratch.store, &mut live, &batch).expect("commit");
    assert_eq!(live.pools[&pool].total_staked, 6_000);

    // Establish that this event really does accumulate, so the assertion below is
    // about the applied set and not about `Staked` happening to be assignment-like.
    let mut naive = Analytics::new();
    for _ in 0..2 {
        for tx in &batch {
            naive.apply_transaction(&tx.signature, &tx.events);
        }
    }
    assert_eq!(
        naive.pools[&pool].total_staked, 6_000,
        "even a fresh projection deduplicates within one process — this fixture \
         cannot demonstrate the hazard"
    );
    let mut doubled = Analytics::new();
    doubled.apply_transaction("sig-a", &batch[0].events);
    doubled.apply_transaction("sig-a-again", &batch[0].events);
    assert_eq!(
        doubled.pools[&pool].total_staked, 12_000,
        "`Staked` no longer accumulates, so nothing here is at risk and this test \
         has stopped testing its claim"
    );

    // The restart.
    let restored = scratch.reopen().load().expect("load");
    assert_eq!(restored.cursor.slot, 900);
    let ingestor = Ingestor::restore(restored.cursor, restored.state);

    // The source serves slot 900 again. The projection must recognise it.
    let mut replayed = ingestor.finalized().clone();
    let applied = replayed.apply_transaction("sig-a", &batch[0].events);

    assert_eq!(
        applied, 0,
        "a restored projection re-folded events it had already applied"
    );
    assert_eq!(
        replayed.pools[&pool].total_staked, 6_000,
        "the redelivered stake was counted twice across the restart"
    );
}

/// The `updated_at_slot` guard, which is the whole reason two writers are safe.
#[test]
fn an_older_slot_cannot_overwrite_a_newer_row() {
    let mut scratch = store_or_skip!("outoforder");

    let (pool, position) = (Pubkey::new_unique(), Pubkey::new_unique());

    // The live stream, at slot 500.
    let mut live = Analytics::new();
    apply_and_commit(
        &mut scratch.store,
        &mut live,
        &[settled(
            500,
            "sig-live",
            vec![
                pool_initialized(pool, Pubkey::new_unique()),
                staked(pool, position, Pubkey::new_unique(), 9_000),
            ],
        )],
    )
    .expect("live commit");

    // A backfill working through slot 200 with a view of the pool built from
    // older history. Its figures are not wrong, they are stale — which is exactly
    // what makes overwriting them silent.
    let mut backfill = Analytics::new();
    let stale = apply_and_commit(
        &mut scratch.store,
        &mut backfill,
        &[settled(
            200,
            "sig-backfill",
            vec![staked(pool, Pubkey::new_unique(), Pubkey::new_unique(), 1)],
        )],
    )
    .expect("backfill commit");

    assert_eq!(stale, 1, "the backfill wrote no event row");
    assert_ne!(
        backfill.pools[&pool].total_staked, live.pools[&pool].total_staked,
        "the two writers agree, so the guard is not being exercised"
    );

    let after = scratch.reopen().load().expect("load");
    assert_eq!(
        after.state.pools[&pool].total_staked, 9_000,
        "a backfill replaying an older slot overwrote the newer live figure"
    );

    // The event row itself is history and belongs there whatever its slot.
    assert_eq!(
        scratch.count("events"),
        3,
        "the backfill's event was rejected along with its projection write"
    );
}

/// A crash mid-commit must not leave the cursor ahead of the rows.
///
/// This is the failure that cannot be recovered from: resume at slot 20 with slot
/// 20's events unwritten and the ingestor will never ask for them again, because
/// as far as it is concerned they are behind it. Nothing reports it, and the
/// figures are merely a little too small forever.
///
/// The injection is a dropped table. Anything that makes one statement fail part
/// way through the sequence proves the same thing, and dropping `votes` puts the
/// failure *after* the event rows and several projection tables have been
/// written — so a binding without a transaction around them would leave exactly
/// the torn state this asserts is impossible.
#[test]
fn a_failed_commit_leaves_the_cursor_and_the_rows_where_they_were() {
    let mut scratch = store_or_skip!("atomicity");

    let (pool, realm, proposal) = (
        Pubkey::new_unique(),
        Pubkey::new_unique(),
        Pubkey::new_unique(),
    );
    let position = Pubkey::new_unique();

    let mut state = Analytics::new();
    apply_and_commit(
        &mut scratch.store,
        &mut state,
        &[settled(
            10,
            "sig-first",
            vec![
                pool_initialized(pool, Pubkey::new_unique()),
                HelixEvent::ProposalCreated(helix_governance::events::ProposalCreated {
                    realm,
                    proposal,
                    proposer: Pubkey::new_unique(),
                    id: 0,
                    action: helix_governance::state::ProposalAction::Signal,
                    title: "t".into(),
                    timestamp: 1,
                }),
            ],
        )],
    )
    .expect("first commit");

    scratch
        .raw()
        .batch_execute("DROP TABLE votes")
        .expect("drop votes");

    let doomed = vec![settled(
        20,
        "sig-torn",
        vec![
            // Written first, so its absence afterwards is the rollback.
            staked(pool, position, Pubkey::new_unique(), 3_000),
            HelixEvent::VoteCast(helix_governance::events::VoteCast {
                proposal,
                position,
                voter: Pubkey::new_unique(),
                choice: helix_governance::state::VoteChoice::For,
                weight: 3_000,
                for_votes: 3_000,
                against_votes: 0,
                abstain_votes: 0,
                timestamp: 2,
            }),
        ],
    )];

    let error = apply_and_commit(&mut scratch.store, &mut state, &doomed)
        .expect_err("committing into a dropped table succeeded");
    assert!(
        matches!(error, StoreError::Query { .. }),
        "unexpected failure: {error:?}"
    );

    // Recreate the table so `load` can run — the assertion is about what is in
    // the other tables, and votes is empty either way.
    scratch
        .raw()
        .batch_execute(
            "CREATE TABLE votes (proposal TEXT NOT NULL REFERENCES proposals (address), \
             position TEXT NOT NULL, voter TEXT NOT NULL, choice TEXT NOT NULL, \
             weight NUMERIC(20,0) NOT NULL, voted_at BIGINT NOT NULL, \
             PRIMARY KEY (proposal, position))",
        )
        .expect("recreate votes");

    let after = scratch.reopen().load().expect("load");
    assert_eq!(
        after.cursor.slot, 10,
        "the cursor moved past a batch that was not written"
    );
    assert_eq!(
        scratch.count("events"),
        2,
        "the failed batch left event rows behind"
    );
    assert!(
        !after.state.positions.contains_key(&position),
        "a position from the failed batch was persisted"
    );
    assert_eq!(
        after.state.pools[&pool].total_staked, 0,
        "the pool absorbed a stake from a batch that failed to commit"
    );
}

/// Closing a position deallocates the account, so the row has to go with it.
#[test]
fn a_closed_position_leaves_no_row_but_keeps_the_electorate_boundary() {
    let mut scratch = store_or_skip!("close");

    let (pool, position, owner) = (
        Pubkey::new_unique(),
        Pubkey::new_unique(),
        Pubkey::new_unique(),
    );

    let mut state = Analytics::new();
    apply_and_commit(
        &mut scratch.store,
        &mut state,
        &[settled(
            1,
            "sig-open",
            vec![
                pool_initialized(pool, Pubkey::new_unique()),
                staked(pool, position, owner, 2_000),
            ],
        )],
    )
    .expect("open");
    assert_eq!(scratch.count("positions"), 1);

    apply_and_commit(
        &mut scratch.store,
        &mut state,
        &[settled(
            2,
            "sig-close",
            vec![
                HelixEvent::Unstaked(helix_staking::events::Unstaked {
                    pool,
                    position,
                    owner,
                    amount: 2_000,
                    remaining: 0,
                    weighted_amount: 0,
                    timestamp: 3,
                }),
                HelixEvent::PositionClosed(helix_staking::events::PositionClosed {
                    pool,
                    position,
                    owner,
                    position_id: 0,
                    timestamp: 3,
                }),
            ],
        )],
    )
    .expect("close");

    assert_eq!(
        scratch.count("positions"),
        0,
        "the row outlived the account it mirrors"
    );

    let after = scratch.reopen().load().expect("load");
    // `position_count` counts positions ever opened — it is the boundary
    // governance snapshots as the electorate, so decrementing it on close would
    // make the store disagree with the chain about who could vote. F-10.
    assert_eq!(after.state.pools[&pool].position_count, 1);
    assert_eq!(
        scratch.count("events"),
        4,
        "history lost the closed position"
    );
}

/// `BIGINT` is signed, and a u64 balance can exceed it.
///
/// Read back with a plain client and compared as text, so this fails if the value
/// was ever narrowed to an i64 on either leg — a round trip through the binding
/// alone would agree with itself about a wrapped number.
#[test]
fn an_amount_above_the_signed_range_survives_the_database() {
    let mut scratch = store_or_skip!("u64");

    let (pool, position) = (Pubkey::new_unique(), Pubkey::new_unique());
    let huge = u64::MAX - 7;
    assert!(huge > i64::MAX as u64, "the fixture is inside i64");

    let mut state = Analytics::new();
    apply_and_commit(
        &mut scratch.store,
        &mut state,
        &[settled(
            1,
            "sig",
            vec![
                pool_initialized(pool, Pubkey::new_unique()),
                HelixEvent::RewardsFunded(helix_staking::events::RewardsFunded {
                    pool,
                    funder: Pubkey::new_unique(),
                    amount_credited: huge,
                    total_funded: huge,
                    timestamp: 1,
                }),
                staked(pool, position, Pubkey::new_unique(), 1),
            ],
        )],
    )
    .expect("commit");

    let stored: String = scratch
        .raw()
        .query_one(
            "SELECT total_rewards_funded::text FROM pools WHERE address = $1",
            &[&pool.to_string()],
        )
        .expect("read")
        .get(0);
    assert_eq!(
        stored,
        huge.to_string(),
        "the amount was narrowed on the way in"
    );

    let after = scratch.reopen().load().expect("load");
    assert_eq!(after.state.pools[&pool].total_rewards_funded, huge);
}

/// Nothing unfinalised reaches the database.
///
/// Driven through a real `Ingestor` rather than hand-built batches, because the
/// claim is about where the boundary sits in the ingestion path, not about what
/// `commit` does with what it is handed.
#[test]
fn only_finalised_transactions_are_written() {
    let mut scratch = store_or_skip!("finality");

    let pool = Pubkey::new_unique();
    let mut source = helix_indexer::source::ScriptedSource::new();
    for slot in 1..=4u64 {
        source.push(
            &format!("sig-{slot}"),
            slot,
            log_lines(vec![
                pool_initialized(pool, Pubkey::new_unique()),
                staked(pool, Pubkey::new_unique(), Pubkey::new_unique(), 100),
            ]),
        );
    }
    // Slots 3 and 4 are still revocable.
    source.finalize_through(2);

    let mut ingestor = Ingestor::new();
    let outcome = ingestor.poll(&mut source, 100).expect("poll");
    assert_eq!(outcome.applied, 4, "head did not see the unfinalised tail");
    assert_eq!(outcome.finalized(), 2);

    scratch
        .store
        .commit(ingestor.cursor(), ingestor.finalized(), &outcome.settled)
        .expect("commit");

    let signatures: Vec<String> = scratch
        .raw()
        .query(
            "SELECT DISTINCT signature FROM events ORDER BY signature",
            &[],
        )
        .expect("read")
        .iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(
        signatures,
        vec!["sig-1".to_string(), "sig-2".to_string()],
        "a transaction a fork could still take back was persisted"
    );

    let after = scratch.reopen().load().expect("load");
    assert_eq!(after.cursor.slot, 2);
    assert_eq!(
        after.state.pools[&pool].total_staked, 200,
        "the stored projection is head rather than finalized"
    );
    assert_eq!(
        ingestor.head().pools[&pool].total_staked,
        400,
        "the in-memory head should still hold all four"
    );
}

/// Two binaries at different schema versions must not share a database.
#[test]
fn a_database_at_another_schema_version_is_refused() {
    let scratch = store_or_skip!("version");

    scratch
        .raw()
        .batch_execute("UPDATE schema_version SET version = 99")
        .expect("restamp");

    let error = scratch
        .reopen()
        .migrate()
        .expect_err("a database from the future was accepted");
    assert!(
        matches!(error, StoreError::SchemaVersion { found: 99, .. }),
        "unexpected error: {error:?}"
    );
}

/// A truncated log is a fact about a transaction, and it is stored with one.
#[test]
fn an_anomaly_is_stored_with_the_signature_that_makes_it_actionable() {
    let mut scratch = store_or_skip!("anomaly");

    let pool = Pubkey::new_unique();
    let mut source = helix_indexer::source::ScriptedSource::new();
    let mut lines = log_lines(vec![pool_initialized(pool, Pubkey::new_unique())]);
    lines.insert(1, "Log truncated".into());
    source.push("sig-truncated", 1, lines);
    source.finalize_through(1);

    let mut ingestor = Ingestor::new();
    let outcome = ingestor.poll(&mut source, 10).expect("poll");
    assert!(
        !outcome.settled[0].anomalies.is_empty(),
        "the fixture produced no anomaly, so nothing is being stored"
    );

    scratch
        .store
        .commit(ingestor.cursor(), ingestor.finalized(), &outcome.settled)
        .expect("commit");

    let row = scratch
        .raw()
        .query_one("SELECT signature, kind FROM ingestion_anomalies", &[])
        .expect("one anomaly row");
    let (signature, kind): (String, String) = (row.get(0), row.get(1));
    assert_eq!(signature, "sig-truncated");
    assert_eq!(kind, "truncated");
}

/// Anchor's wire form for a set of events, as the runtime would log them.
fn log_lines(events: Vec<HelixEvent>) -> Vec<String> {
    use anchor_lang::{AnchorSerialize, Discriminator};
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine as _;

    let mut lines = vec![format!("Program {} invoke [1]", helix_staking::ID)];
    for event in events {
        let bytes = match event {
            HelixEvent::PoolInitialized(e) => {
                let mut bytes = helix_staking::events::PoolInitialized::DISCRIMINATOR.to_vec();
                e.serialize(&mut bytes).expect("serialize");
                bytes
            }
            HelixEvent::Staked(e) => {
                let mut bytes = helix_staking::events::Staked::DISCRIMINATOR.to_vec();
                e.serialize(&mut bytes).expect("serialize");
                bytes
            }
            other => panic!("log_lines does not build {}", other.name()),
        };
        lines.push(format!("Program data: {}", BASE64.encode(bytes)));
    }
    lines.push(format!("Program {} success", helix_staking::ID));
    lines
}
