-- Persistent form of the projection in `src/projection.rs`.
--
-- Executed by `Store::migrate` in `src/store.rs`, which reads this file with
-- `include_str!` — so the schema the tests run against is this text, not a
-- second copy of it that resembles it.
--
-- The rule it is built around: ingestion must be safe to repeat. Confirmed logs
-- get redelivered, a backfill overlaps a live stream, and a reorg makes both
-- happen at once. Every write below is therefore either an idempotent insert or
-- an upsert whose result does not depend on how many times it ran.
--
-- The second rule, which only became visible once something executed the first:
-- **nothing is written until it is final.** See `events` below.

-- --------------------------------------------------------------- schema version
--
-- Not a migration system. `migrate()` creates what is missing and refuses a
-- database stamped with a version it does not know; it does not alter an
-- existing table to a new shape. That is the honest boundary for a single-file
-- schema, and it catches the failure this would otherwise have — two binaries at
-- different versions pointed at one database, the older one writing rows with
-- columns it has never heard of left at their defaults.

CREATE TABLE IF NOT EXISTS schema_version (
    -- One row, enforced.
    id       BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (id),
    version  INTEGER NOT NULL,
    applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO schema_version (version) VALUES (1) ON CONFLICT (id) DO NOTHING;

-- ---------------------------------------------------------------- raw events
--
-- The append-only log. Everything else in this file is derivable from it, so a
-- projection bug is fixed by replaying rather than by re-fetching the chain.
--
-- **Only finalised transactions land here.** An earlier draft of this file had an
-- `orphaned_at` column for "rows whose slot was rolled back", which reads well and
-- could never have been written: the binding persists the `finalized` projection
-- and its cursor, and finalised history does not change — a contradiction below
-- the watermark stops ingestion outright (`IngestError::FinalizedHistoryChanged`)
-- rather than producing rows to mark. A column nothing can ever write is a claim
-- about behaviour the system does not have, which is the same defect as an
-- invariant that cannot fail. It is gone.
--
-- The cost of that choice is that the unfinalised tail — tens of slots, seconds of
-- chain — is re-read from the source after a restart instead of being loaded. That
-- is cheap, and it is what makes every stored row unconditionally true.

CREATE TABLE IF NOT EXISTS events (
    signature   TEXT     NOT NULL,
    -- Index of the `Program data:` line within the transaction's log.
    --
    -- Part of the key, not decoration. A signature alone is not unique: one
    -- transaction emits several events, and two of them can be byte-identical —
    -- staking the same amount into the same pool twice in one transaction is a
    -- legitimate thing to do, and deduplicating on content would lose the second.
    log_index   INTEGER  NOT NULL,
    slot        BIGINT   NOT NULL,
    -- CPI depth, 1 for a top-level instruction. Retained because attribution
    -- errors are invisible in the decoded row but obvious next to the depth.
    depth       INTEGER  NOT NULL,
    program     TEXT     NOT NULL,
    kind        TEXT     NOT NULL,
    -- On-chain time from the event body, not ingestion time. The two differ by
    -- however long the indexer was down, and only one of them is history.
    block_time  BIGINT   NOT NULL,
    -- The exact bytes the chain wrote on the `Program data:` line: Anchor's
    -- 8-byte discriminator followed by the Borsh body.
    --
    -- This column was JSONB, and JSONB is the better answer to "what would you
    -- like to query". It is the wrong answer to what this table is *for*. The
    -- header above claims everything else is derivable from these rows, and
    -- derivable means decodable by `HelixEvent::decode` — the same function the
    -- live log goes through. Producing JSON would need a hand-written encoder per
    -- event type, which is a second declaration of the event schema in a crate
    -- whose entire justification is not having one (see `indexer/README.md`).
    -- The two would drift at the first program change, silently, because nothing
    -- compares them.
    --
    -- So the queryable facets are columns — program, kind, slot, block_time —
    -- and the body stays in the form the chain emitted it. Anything wanting to
    -- query inside a payload replays through the decoder into the projection
    -- tables below, which is what those tables are.
    payload     BYTEA    NOT NULL,
    PRIMARY KEY (signature, log_index)
);

CREATE INDEX IF NOT EXISTS events_slot_idx ON events (slot);
CREATE INDEX IF NOT EXISTS events_kind_time_idx ON events (kind, block_time);

-- ------------------------------------------------------------- ingestion state

-- Where the subscriber and the backfill have each reached. Two cursors, because
-- they move in opposite directions and must not overwrite one another.
CREATE TABLE IF NOT EXISTS cursors (
    name            TEXT PRIMARY KEY,   -- 'live' | 'backfill'
    slot            BIGINT NOT NULL,
    signature       TEXT,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Everything the ingestion path saw that means the rows above are not the whole
-- story. Recorded rather than dropped: an indexer that silently discards what it
-- cannot read produces wrong numbers precisely when something unusual happened.
--
-- `signature` is what makes a row actionable — "log truncated at line 12" with no
-- transaction to re-fetch is a statistic, not a report. It was in this table's key
-- from the start and the code could not supply it until Phase 4.2; see
-- `ReportedAnomaly` in `src/ingest.rs`.
CREATE TABLE IF NOT EXISTS ingestion_anomalies (
    signature   TEXT NOT NULL,
    log_index   INTEGER NOT NULL,
    -- 'truncated'  — the runtime cut the log off; events after it were never written
    -- 'undecodable'— a payload this build does not know, i.e. the indexer is older
    --                than the chain
    -- 'unbalanced' — the invoke stack did not close, so attribution below it is
    --                not trustworthy
    -- 'orphaned'   — the event referenced an entity this stream never saw created.
    --                Expected on a backfill that starts mid-history; on a live
    --                stream it means something was dropped.
    kind        TEXT NOT NULL,
    detail      TEXT,
    seen_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (signature, log_index, kind)
);

-- ----------------------------------------------------------------- projections

-- NUMERIC(20, 0), not BIGINT, throughout. Solana amounts are u64 and BIGINT is
-- signed 64-bit: a balance above 2^63 overflows it. That is not hypothetical for
-- a 9-decimal token — it is 9.2 billion whole tokens.
--
-- The Rust driver has the mirror image of the same problem: `postgres` maps
-- BIGINT to i64 and NUMERIC to nothing at all, so the obvious binding hands a u64
-- across as an i64 and reintroduces exactly the overflow this column type exists
-- to prevent. `store.rs` passes every amount as text and casts — `$1::text::numeric`
-- going in, `column::text` coming out — which is the same decision, for the same
-- reason, that the read API makes when it serialises amounts as JSON strings.

CREATE TABLE IF NOT EXISTS realms (
    address                 TEXT PRIMARY KEY,
    authority               TEXT,
    guardian                TEXT,
    staking_pool            TEXT,
    -- What "passing" means. A vote tally stored without these is a numerator with
    -- no denominator.
    quorum_bps              INTEGER        NOT NULL DEFAULT 0,
    approval_bps            INTEGER        NOT NULL DEFAULT 0,
    voting_period           BIGINT         NOT NULL DEFAULT 0,
    timelock_delay          BIGINT         NOT NULL DEFAULT 0,
    min_weight_to_propose   NUMERIC(20, 0) NOT NULL DEFAULT 0,
    -- True once the realm's parameters answer only to the realm itself. The one
    -- fact this whole protocol is built to reach — ROADMAP Phase 7.
    self_governing          BOOLEAN        NOT NULL DEFAULT FALSE,
    partial_history         BOOLEAN        NOT NULL DEFAULT FALSE,
    updated_at_slot         BIGINT         NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS pools (
    address                 TEXT PRIMARY KEY,
    authority               TEXT,
    total_staked            NUMERIC(20, 0) NOT NULL DEFAULT 0,
    total_weighted          NUMERIC(20, 0) NOT NULL DEFAULT 0,
    position_count          BIGINT         NOT NULL DEFAULT 0,
    reward_rate             NUMERIC(20, 0) NOT NULL DEFAULT 0,
    reward_period_end       BIGINT         NOT NULL DEFAULT 0,
    total_rewards_funded    NUMERIC(20, 0) NOT NULL DEFAULT 0,
    total_rewards_paid      NUMERIC(20, 0) NOT NULL DEFAULT 0,
    paused                  BOOLEAN        NOT NULL DEFAULT FALSE,
    -- True when the first event touching this pool was not its initialisation,
    -- meaning the stream started mid-history. The figures are the best available
    -- but are not complete, and a caller is entitled to know which it has.
    partial_history         BOOLEAN        NOT NULL DEFAULT FALSE,
    updated_at_slot         BIGINT         NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS positions (
    address             TEXT PRIMARY KEY,
    pool                TEXT NOT NULL REFERENCES pools (address),
    owner               TEXT NOT NULL,
    position_id         BIGINT NOT NULL,
    amount              NUMERIC(20, 0) NOT NULL DEFAULT 0,
    weighted_amount     NUMERIC(20, 0) NOT NULL DEFAULT 0,
    tier                TEXT NOT NULL,
    lock_end            BIGINT NOT NULL,
    rewards_claimed     NUMERIC(20, 0) NOT NULL DEFAULT 0,
    updated_at_slot     BIGINT NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS positions_owner_idx ON positions (owner);
-- Serves the staker distribution, which excludes fully withdrawn positions: the
-- account still exists on chain but a zero-weight position is not a staker.
CREATE INDEX IF NOT EXISTS positions_pool_live_idx ON positions (pool) WHERE amount > 0;

-- Deliberately no `REFERENCES realms`. A proposal is projected from its own
-- events, and a stream that starts mid-history sees `ProposalCreated` without ever
-- seeing the realm's initialisation. A foreign key here would turn "we joined late"
-- into a failed write.
CREATE TABLE IF NOT EXISTS proposals (
    address                 TEXT PRIMARY KEY,
    realm                   TEXT   NOT NULL,
    proposal_id             BIGINT NOT NULL,
    proposer                TEXT   NOT NULL,
    title                   TEXT   NOT NULL,
    state                   TEXT   NOT NULL,
    for_votes               NUMERIC(20, 0) NOT NULL DEFAULT 0,
    against_votes           NUMERIC(20, 0) NOT NULL DEFAULT 0,
    abstain_votes           NUMERIC(20, 0) NOT NULL DEFAULT 0,
    total_weight_snapshot   NUMERIC(20, 0) NOT NULL DEFAULT 0,
    -- How many positions that snapshot covers. Both halves are needed to say
    -- whether a later vote came from the electorate, and the chain stores both.
    position_count_snapshot BIGINT NOT NULL DEFAULT 0,
    eta                     BIGINT,
    updated_at_slot         BIGINT NOT NULL DEFAULT 0,
    UNIQUE (realm, proposal_id)
);

CREATE TABLE IF NOT EXISTS votes (
    proposal    TEXT NOT NULL REFERENCES proposals (address),
    position    TEXT NOT NULL,
    voter       TEXT NOT NULL,
    choice      TEXT NOT NULL,
    weight      NUMERIC(20, 0) NOT NULL,
    voted_at    BIGINT NOT NULL,
    -- One vote per (proposal, position), matching the on-chain VoteRecord PDA.
    -- The constraint is the same one the program enforces by `init`, restated
    -- here so a replay cannot produce a second row.
    PRIMARY KEY (proposal, position)
);

CREATE TABLE IF NOT EXISTS treasuries (
    address                 TEXT PRIMARY KEY,
    governance_executor     TEXT,
    total_deposited         NUMERIC(20, 0) NOT NULL DEFAULT 0,
    total_spent             NUMERIC(20, 0) NOT NULL DEFAULT 0,
    total_stream_claims     NUMERIC(20, 0) NOT NULL DEFAULT 0,
    epoch_spend_cap         NUMERIC(20, 0) NOT NULL DEFAULT 0,
    partial_history         BOOLEAN NOT NULL DEFAULT FALSE,
    updated_at_slot         BIGINT NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS streams (
    address         TEXT PRIMARY KEY,
    treasury        TEXT NOT NULL REFERENCES treasuries (address),
    stream_id       BIGINT NOT NULL,
    beneficiary     TEXT NOT NULL,
    total_amount    NUMERIC(20, 0) NOT NULL,
    claimed         NUMERIC(20, 0) NOT NULL DEFAULT 0,
    revoked         BOOLEAN NOT NULL DEFAULT FALSE,
    updated_at_slot BIGINT NOT NULL DEFAULT 0,
    UNIQUE (treasury, stream_id)
);

-- ------------------------------------------------------------------ upserts
--
-- The shape every projection write takes. `store.rs` builds these; they are
-- restated here because the guard is the whole design and it is one line.
--
-- Running totals are assigned wherever the chain publishes one — `total_funded`,
-- `total_spent`, `total_claimed` — so assignment converges to the right answer no
-- matter how many times a row is replayed, and recovers the correct figure even
-- when earlier history was missed.
--
-- Four events do not get that luxury: `Staked`, `Unstaked`, `RewardsClaimed` and
-- `StreamClaimed` publish a delta and no running total, so the projection
-- genuinely accumulates for those. Their idempotency comes entirely from the
-- `events` table above being the dedup key, which is why a projection loaded from
-- storage must load the applied set with it — see `Analytics::mark_applied`.
--
--   INSERT INTO pools (address, total_rewards_funded, updated_at_slot)
--   VALUES ($1, $2::text::numeric, $3)
--   ON CONFLICT (address) DO UPDATE
--      SET total_rewards_funded = EXCLUDED.total_rewards_funded,
--          updated_at_slot      = EXCLUDED.updated_at_slot
--    WHERE pools.updated_at_slot <= EXCLUDED.updated_at_slot;
--
-- The WHERE clause is what makes out-of-order delivery safe: a backfill
-- replaying an old slot cannot overwrite a newer live update. `<=` rather than
-- `<` because two events in the same slot must both land, in order.
