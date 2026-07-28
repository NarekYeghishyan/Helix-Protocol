-- Persistent form of the projection in `src/projection.rs`.
--
-- STATUS: written, not yet exercised. Nothing connects to a database yet — see
-- indexer/README.md. It is here because the shape of the schema is a design
-- decision worth reviewing before the binding is written, not because it runs.
--
-- The rule it is built around: ingestion must be safe to repeat. Confirmed logs
-- get redelivered, a backfill overlaps a live stream, and a reorg makes both
-- happen at once. Every write below is therefore either an idempotent insert or
-- an upsert whose result does not depend on how many times it ran.

-- ---------------------------------------------------------------- raw events

-- The append-only log. Everything else in this file is derivable from it, so a
-- projection bug is fixed by replaying rather than by re-fetching the chain.
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
    payload     JSONB    NOT NULL,
    PRIMARY KEY (signature, log_index)
);

CREATE INDEX IF NOT EXISTS events_slot_idx ON events (slot);
CREATE INDEX IF NOT EXISTS events_kind_time_idx ON events (kind, block_time);

-- Rows whose slot was rolled back. Marked rather than deleted: a reorg that
-- removes a transaction is itself a fact worth keeping, and silent deletion
-- makes "the number changed" indistinguishable from "the number was wrong".
ALTER TABLE events ADD COLUMN IF NOT EXISTS orphaned_at TIMESTAMPTZ;

-- ------------------------------------------------------------- ingestion state

-- Where the subscriber and the backfill have each reached. Two cursors, because
-- they move in opposite directions and must not overwrite one another.
CREATE TABLE IF NOT EXISTS cursors (
    name            TEXT PRIMARY KEY,   -- 'live' | 'backfill'
    slot            BIGINT NOT NULL,
    signature       TEXT,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Transactions whose logs could not be fully read — truncated, or carrying a
-- discriminator this build does not know. Recorded so they can be re-fetched
-- with inner instructions rather than silently contributing nothing.
CREATE TABLE IF NOT EXISTS ingestion_anomalies (
    signature   TEXT NOT NULL,
    log_index   INTEGER NOT NULL,
    kind        TEXT NOT NULL,          -- 'truncated' | 'undecodable' | 'unbalanced'
    detail      TEXT,
    seen_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (signature, log_index, kind)
);

-- ----------------------------------------------------------------- projections

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

-- NUMERIC(20, 0), not BIGINT, throughout. Solana amounts are u64 and BIGINT is
-- signed 64-bit: a balance above 2^63 overflows it. That is not hypothetical for
-- a 9-decimal token — it is 9.2 billion whole tokens.

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
-- Illustrative rather than exhaustive: the shape every projection write takes.
--
-- Running totals are assigned, never accumulated. The events carry them
-- (`total_funded`, `total_spent`, `total_claimed`), so assignment converges to
-- the right answer no matter how many times a row is replayed — and it recovers
-- the correct figure even when earlier history was missed. Accumulating with
-- `+=` would be correct only if delivery were exactly-once, which it is not.

-- INSERT INTO pools (address, total_rewards_funded, updated_at_slot)
-- VALUES ($1, $2, $3)
-- ON CONFLICT (address) DO UPDATE
--    SET total_rewards_funded = EXCLUDED.total_rewards_funded,
--        updated_at_slot      = EXCLUDED.updated_at_slot
--  WHERE pools.updated_at_slot <= EXCLUDED.updated_at_slot;
--
-- The WHERE clause is what makes out-of-order delivery safe: a backfill
-- replaying an old slot cannot overwrite a newer live update.
