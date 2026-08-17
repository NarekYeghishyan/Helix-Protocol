# Testing procedures

*Deliverable 5 — testing procedures.*

This document is deliberately explicit about what is **not** covered. A test suite's value
is bounded by the honesty of its coverage claims.

## Running the suite

```bash
anchor build                                                  # required first — the
                                                              # runtime tests load .so files
cargo test --workspace                                        # 254 tests: unit + doc + runtime
cd app && npm test                                            # 72 dashboard tests, no framework
cargo test -p helix-staking --lib                             # one program's unit tests
cargo test -p helix-staking --lib -- --nocapture rounding     # one test, with output

cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check

anchor build 2>&1 | tee build.log
grep -i "stack offset" build.log     # MUST be empty — see below
```

`--all-features` matters: the indexer's `rpc`, `server` and `postgres` modules hold all of
its I/O and are all off by default, so a plain `clippy` never looks at any of them.

### Running the live tests

Eighteen tests need something outside the process — a validator or a database. All of them
**skip and pass** when the relevant environment variable is unset, which is why the count
above is the same either way. That is deliberate: a suite that fails on a clean checkout
trains people to ignore red, and `#[ignore]` hides the reason behind a flag someone has to
already know about.

```bash
solana-test-validator --reset &                # a real cluster, no faucet needed
for p in helix_token_manager helix_staking helix_governance helix_treasury; do
  solana -u localhost program deploy \
    --program-id target/deploy/$p-keypair.json target/deploy/$p.so
done

HELIX_RPC_URL=http://127.0.0.1:8899 \
  cargo test -p helix-integration-tests --test rpc_source_live -- --test-threads=1
```

`--test-threads=1` because they share one ledger, and one of them asserts that a poll
picked up exactly the transactions it sent.

The ten storage tests need Postgres instead, and run in parallel — each takes its own
schema:

```bash
docker run -d --name helix-postgres -e POSTGRES_PASSWORD=helix -e POSTGRES_USER=helix   -e POSTGRES_DB=helix -p 55432:5432 postgres:16-alpine

HELIX_DATABASE_URL=postgres://helix:helix@127.0.0.1:55432/helix   cargo test -p helix-integration-tests --test store_postgres
```

**A local validator is a real cluster.** Devnet deployment is blocked on faucet funding,
but nothing about `getSignaturesForAddress` or `getTransaction` needs devnet specifically —
and a local validator airdrops without limit. What it still cannot do is fork, which is
why reorg handling is tested against a scripted source instead
([`ingest.rs`](../indexer/src/ingest.rs)).

### Two non-obvious steps

**`anchor build` exits 0 on a stack overflow.** It reports SBF stack-frame overflows as
`Error:` and returns success anyway. A program that overflows its 4KB frame may corrupt
memory at runtime, so the exit code is actively misleading. Always grep the log; CI does
the same rather than trusting the status code. See
[F-3](./SECURITY-ASSESSMENT.md#f-3--sbf-stack-frame-overflow).

**`cargo test` does not rebuild the programs.** The runtime tests load `.so` files from
`target/deploy`, and nothing checks that those artifacts came from the source you are
looking at. Edit a program, run `cargo test`, and you are testing the previous build.

That is not a hypothetical. It happened here while mutation-testing `close_position`:
the mutated build stayed in `target/deploy` after the source was reverted, and the next
full run produced six failures in unrelated tests with `Custom(2000)` — `ConstraintSeeds`,
which points at the account list rather than at the actual cause. The rule is to rebuild
before every runtime run, and to distrust a sudden cluster of failures that share an error
code you did not expect.

## What is covered

**135 unit tests** over pure functions and state machines, **98 runtime tests** against the
real BPF programs under LiteSVM, **18 live tests** against a validator and a Postgres
instance, and **72 dashboard tests** under `node --test`. `cargo test --workspace` reports
254, the balance being doctests and the two checks that compare generated build artifacts
against hand-maintained lists — `event_coverage.rs` and `idl_sync.rs`.

The tiers are not a hierarchy but a statement about what each can prove. Unit tests reach
arithmetic and state machines; LiteSVM reaches the wiring between programs; only real
infrastructure reaches the transport and the storage. Each tier is used for the smallest set
of claims that genuinely needs it, and the live tier is the smallest by design — see
[`indexer/src/rpc.rs`](../indexer/src/rpc.rs).

The dashboard tier is the odd one out and is described separately below, because the thing
it has to avoid is different: everything it tests is derived from the IDL, so a test that
compares against the IDL proves only that a file was read twice.

### Unit tests

| Area | Tests | What is asserted |
|---|---|---|
| Reward accumulator | 12 | Monotonicity, idempotence within a timestamp, emissions halting at period end, clock never running backwards, no yield for time before a position existed, zero yield for open-and-close in one slot |
| Reward rounding | 3 | Differential check against exact arithmetic: the sum payable to all stakers never exceeds what was emitted; dust weight earns zero rather than being rounded up |
| Reward liability | 4 | A funded pool can set a non-zero rate; accrued rewards count against new rates; claimed rewards stop counting; liability is never *understated* |
| Lock tiers | 3 | Weight application, truncation direction, overflow rejection |
| Lock gate | 3 | Fresh positions cannot vote; a lock expiring mid-vote cannot vote; the unlock boundary is inclusive |
| Issuance caps | 7 | Accrual within cap, rejection without mutation, epoch rollover, and that ten idle epochs do not grant ten epochs of allowance |
| Vote tally | 11 | Quorum counting abstentions, cross-multiplied thresholds losing nothing to rounding, unanimous-below-quorum defeated, all-abstain approving nothing, supermajority boundaries |
| Proposal lifecycle | 6 | Guardian veto window, terminal states, state gating, queued-proposal expiry |
| Vesting | 9 | Cliff behaviour, linearity, truncation direction, and the three revocation properties |
| Spend budget | 5 | Cap enforcement, rejection without mutation, rollover, no allowance accumulation |
| Event decoding | 7 | Wire-format round trip, truncated and trailing-byte payloads rejected, decoding scoped to the emitting program |
| Log attribution | 10 | CPI depth tracking, foreign programs ignored, truncation and undecodable payloads reported, compute lines not mistaken for frame exits |
| Projection | 6 | Idempotent replay, identical events in one transaction kept distinct, orphan tracking, APR undefined on an empty pool |
| Ingestion | 9 | Reorg above the finality watermark reverted and replaced, contradiction below it refused, paged backfill equals a single pass, cursor resumption, anomalies surfaced |
| Descent | 8 | A backfill reaches genesis and stops there; page size does not change the history it yields; it resumes from a persisted position; it refuses the unfinalised tail and moves past it anyway; a page with nothing to fold is not genesis; a source that does not descend is refused; a replayed descent equals a forward pass, and folding one in *arrival* order does not |
| Deployment plan | 9 | The payer ends up controlling nothing, addresses derive from the mint alone, the transaction fits a packet, the JSON form keeps signer and writable flags, and the post-deploy audit names every authority that drifted — including the guardian |
| Position closing | 3 | An empty position is closable and a weight-bearing one is not; settling an empty position is a no-op at any accumulator value, which is why `close_position` needs no settlement step |
| Read API | 6 | The two finality views differ and each says which it is, a u64 past 2^53 survives a JSON round trip, undefined APR is null, small shares do not round away |
| RPC source | 4 | A JSON-RPC error object is an error despite the HTTP 200; a null `logMessages` and an unknown signature each decode as *absent* rather than as a transaction that emitted nothing; every program is followed by default |
| Storage binding | 6 | A commit rewrites only the entities its batch touched, and at the *highest* slot it touched them; the stored payload decodes back to the event it came from; every lock tier and proposal state survives its `Debug`-name round trip and an unknown one is an error rather than a default; a u64 past 2^63 survives as text |

### Runtime tests

| File | Tests | What is asserted |
|---|---|---|
| `smoke.rs` | 2 | All four programs load and are executable; IDs are distinct |
| `staking_transfer_fee.rs` | 4 | §1.1, §1.3, §2.1–§2.3 against a real 1% transfer-fee mint |
| `staking_lifecycle.rs` | 12 | §1.2, §1.4, §6.1, §6.2, §6.4, §6.5 — funding, rate solvency, accrual, claim, partial and full unstake, pause semantics, cross-owner claim refused |
| `governance_e2e.rs` | 15 | §4.1–4.7, §4.11–4.13, §5.1 — the authority chain plus one negative test per threat-model attack |
| `vesting_e2e.rs` | 12 | §1.5, §1.6, §7.5, §7.7–7.9 — grant → cliff → claim → revoke, forward-only revocation, committed balance protection, executor migration |
| `bootstrap_atomicity.rs` | 6 | F-1's mitigation: the bootstrap fits one transaction (748 B / 17 accounts, asserted against the 1232-byte limit), re-initialisation fails afterwards, and §5.8 — the post-deploy audit run against a clean system *and* against one whose pool really was front-run |
| `authority_invariants.rs` | 4 | §5.3, §5.5 — the mint's authorities are the PDA and no key present at creation can mint; every stored bump is canonical, and a non-canonical derivation of the vault authority is refused |
| `position_close.rs` | 6 | F-7 and §5.11 — rent returns to the owner, principal and unclaimed rewards each block the close, a closed id is never reused, and a voter cannot exit under a live proposal |
| `token_admin_e2e.rs` | 8 | §5.2, §5.4, §5.9, §5.10 — the token-manager admin handover in real deployment order, and that governance then holds every admin power |
| `compute_budget.rs` | 5 | §6.3 — compute measured across a 64× sweep in staker and voter count, plus a budget ceiling on every hot-path instruction |
| `fuzz_invariants.rs` | 7 | §1.1–1.4, §3.1–3.2, §4.1, §4.3, §4.5–4.6 asserted after every operation of 22 random sequences, plus the tests that keep the campaign honest |
| `realm_authority.rs` | 6 | §4.14, §4.15 — F-11: the realm's parameters reachable by proposal, the human authority revocable, and the attack that was possible before both |
| `indexer_reconciliation.rs` | 8 | The [indexer's](../indexer) projection compared to on-chain accounts field by field, over the staking lifecycle, a fee-bearing mint, the governance lifecycle including nested CPI, replay, and partial history |
`idl_sync.rs` also lives in this crate but is not a runtime test — it loads no program. It is
here because this is the job with `anchor build` output, and it asserts that the dashboard's
checked-in IDLs are what `anchor build` now generates, target the declared program ids, and
still declare every instruction the write flows call. Same placement, and the same reasoning,
as `event_coverage.rs`.

### Dashboard tests

`cd app && npm test` — 72 tests under `node --test`, with Node's native type-stripping. No
test framework is installed and there is no build step.

The dashboard builds transactions from the IDL, which makes the obvious test worthless: the
encoder reads the IDL, so asserting that its output matches the IDL proves a file was opened
twice. Every assertion is independent of that path in one of three ways.

| Way | Where | What it catches |
|---|---|---|
| Anchor's own discriminator rule, recomputed — `sha256("global:<ix>")[0..8]` | `coder.test.ts` | A discriminator the dispatcher would route elsewhere. This checks the recorded value against the rule the runtime actually applies |
| Byte vectors written out by hand | `coder.test.ts` | `stake(7, 1_000_000, Gold)` has one correct encoding. If the coder and the IDL agreed on something wrong, this still fails |
| Account lists transcribed from the `#[derive(Accounts)]` structs | `actions.test.ts` | A builder bug *and* an IDL that has stopped describing the program |

The third is the one that earns its keep, because the hazard it covers is silent.

| File | Tests | What is asserted |
|---|---|---|
| `amount.test.ts` | 8 | Formatting past 2^53, truncation direction, and that the value genuinely does not survive a `Number` — so the test cannot go vacuous |
| `api.test.ts` | 7 | The read-API client against a stub HTTP server the test starts, rather than a patched `fetch` |
| `coder.test.ts` | 16 | Every discriminator in both IDLs against Anchor's rule; exact instruction bytes; a `u64` argument refusing a `number`; a `Position` decoded from bytes the coder did not write; a `Pool` refused as a `Position`; a `Proposal` decoded past its variable-length fields |
| `actions.test.ts` | 21 | The five flows account for account, including that `stake` and `unstake` order the vault and the owner's token account *differently*; both vote gates at their boundaries; the four lifecycle transitions requiring no signer and writing only the proposal; the timelock and its grace period; a non-canonical pool refused; a derivable PDA refusing to be overridden |
| `errors.test.ts` | 10 | Codes worked out from the `#[error_code]` enums; the 6000-in-declaration-order rule `errors.rs` documents; attribution to the program the logs blamed; system-program error 0 named |
| `chain.test.ts` | 7 | The account field lists the UI reads, in layout order — a cast is not a check, and a renamed field compiles fine while every figure derived from it becomes `undefined` |
| `events.test.ts` | 5 | The claimed amount surviving base64 exact past 2^53; `amount_sent` versus `amount_credited`; data lines from other programs ignored rather than mistaken for events |

**The seam and its guard.** The IDLs live in `app/src/idl/` as a copy, because `target/` is
gitignored and the dashboard's CI job has no Anchor toolchain. `idl_sync.rs` runs in the job
that does have it. Both layers were checked by mutation: swapping `stake_vault` and
`owner_token_account` in the copied IDL fails `idl_sync.rs` and, independently,
`actions.test.ts`.

That pair is the whole point. A drifted account order does not fail to encode — Anchor's
account list is positional and unnamed on the wire — so it produces a well-formed
transaction naming the right accounts in the wrong slots, and asks a wallet to sign it.

### Live tests

Against real infrastructure, not LiteSVM. Skipped unless the environment names it.

| File | Tests | Needs | What is asserted |
|---|---|---|---|
| `rpc_source_live.rs` | 8 | `HELIX_RPC_URL` | Phase 4.1 — a projection built over `getSignaturesForAddress` + `getTransaction` matches the accounts on chain; ledger order survives *inside* a slot; a rolled-back transaction whose log still carries its events contributes nothing; a cursor advanced by real finality neither replays nor reads as a fork; the finalized watermark is read from the cluster and moves; one transaction reported under three addresses is folded once |
| `store_postgres.rs` | 10 | `HELIX_DATABASE_URL` | Phase 4.2 — a restart reaches the state it saved, field by field across every projection map; a replayed batch changes nothing; a redelivery *after* a restart does not double-count; a backfill at an older slot cannot overwrite a newer live write; a commit that fails part way leaves the cursor and the rows where they were; a closed position's row goes with the account while `position_count` does not; a u64 above 2^63 survives the database; nothing unfinalised is written; a database at another schema version is refused; an anomaly is stored with the signature that makes it actionable |

The RPC set is also where `helix_ops::plan` — the bootstrap an operator sends — is
submitted to something with a mempool for the first time, rather than only executed
in-process.

### Testing conventions worth copying

**Time is a parameter, never read from `Clock`.** Every function that depends on time
takes `now: i64`. Handlers read the clock once and pass it down. This makes the whole
time dimension unit-testable without a validator or clock warping, and it is why the
accumulator tests can cover boundaries like "same timestamp twice" and "stale timestamp"
directly.

**Rejections must not mutate.** Wherever a check can fail after partial computation, the
test asserts state is unchanged on failure — e.g. `rejects_over_cap_without_mutating`,
`spend_over_cap_is_rejected_without_mutating`. A failed instruction that leaves a trace
is a slow-motion accounting bug.

**Assert the direction of every rounding decision.** Truncation is never incidental. Each
division has a test pinning which side benefits, because "off by one lamport" compounds
across every position and every update.

**A measurement with an uncontrolled confound is not a measurement.** Compute figures moved
by 1,500-unit steps between runs because PDA bumps derive from a randomly generated mint,
which is six times the effect being measured. See [Compute cost](#compute-cost).

**Test the predicate the handler evaluates, not just its inputs.** This is the lesson from
[F-2](./SECURITY-ASSESSMENT.md#f-2--reward-liability-computed-from-deposits): both halves
of the solvency guard were individually correct and individually tested, and the defect
lived in their composition — a guard that could never approve any non-zero reward rate.
`solvency_ok()` in the staking tests now mirrors the handler's actual comparison.

## Stateful fuzzing

[`fuzz.rs`](../tests/integration/src/fuzz.rs) generates random operation sequences, runs
them against the real BPF programs, and reads every aggregate invariant back out of the
accounts **after each operation**. Those are the invariants unit tests cannot reach:
`Σ position.amount == vault.amount` needs real positions and a real vault.

```bash
cargo test -p helix-integration-tests --test fuzz_invariants
```

It found [F-10](./SECURITY-ASSESSMENT.md#f-10--post-snapshot-weight-could-vote), a High:
weight staked *after* a proposal's snapshot could vote, inflating the quorum numerator
against a fixed denominator. Every scripted governance test staked its voters before
activating, because that is the order a person writes when describing how governance is
supposed to work. The generator had no such habit.

**Not Trident**, and the reason is checkable: its newest release pins `solana-sdk ^2.3`
while `anchor-lang` 1.1.2 resolves the Solana crates at 3.x — two major versions of the SDK
in one graph, the same breakage that pins `litesvm` to `=0.13.1`.

Three properties make it worth its runtime, each with a test that fails if it stops holding:

| Property | Kept by |
|---|---|
| Deterministic — a seed reproduces a run exactly, keys included | `a_run_is_reproducible_from_its_seed` |
| The oracle would notice — corrupt an account and the right section objects | `the_oracle_notices_corrupted_state` |
| A failure is actionable — delta debugging reduces to the minimal sequence | `the_shrinker_reduces_to_the_minimal_case` |
| Not vacuous — every operation is both accepted and rejected somewhere | `the_fuzzer_is_not_vacuous` |

The oracle also carries **negative** expectations. It tracks which positions have voted and
which proposals have executed, so an operation that *succeeds* when it had to fail is a
violation — §4.1 and §4.5 checked in the direction that matters.

### What it took to make the fuzzer reach anything

Writing the generator was the easy half. The governance lifecycle is six ordered steps
against one proposal, each gated on state and most of them on the clock, so a uniform
generator spends its whole budget bouncing off `InvalidProposalState`. Measured, per
campaign:

| | queue accepted | execute accepted |
|---|---:|---:|
| First working version, 60 ops | 0 | 0 |
| 90 ops | 2 | 0 |
| 120 ops | 3 | 1 |
| 150 ops, with the fixes below | 10 | 3 |

Every invariant passed at 60 operations, and the entire second half of the state machine —
the timelock, expiry, double execution — was untested. **A green fuzz campaign that never
reaches the interesting states is the most expensive way to prove nothing.**

Three changes, each found by reading a coverage table rather than by guessing:

- **State-aware target selection.** `Op::Queue` picks a proposal already in `Succeeded`,
  falling back to any proposal when none is. The fallback is what keeps the guards under
  test; `the_fuzzer_is_not_vacuous` fails if it stops firing.
- **Eligibility-aware voter selection.** Proposals were finalising `Defeated` eleven times
  for every one that survived, because quorum needs roughly a fifth of the pool and each
  vote was cast by a position picked uniformly — usually one that had already voted, was
  flexible-tier, or was staked after the snapshot.
- **A sequence length chosen by measurement.** 150, from the table above. Past that the
  curve flattens and the runtime does not.

The clock needed the same treatment. Governance runs on hours-to-days and stake locks run
on months, and no single random warp distribution serves both: narrow enough to sit inside
a voting window and it never expires a 180-day lock, wide enough to expire one and it steps
clean over every window it passes. Hence `WarpToDeadline` and `WarpToUnlock`, which read
chain state and land *on* a boundary rather than near it.

## Compute cost

Measured by [`compute_budget.rs`](../tests/integration/tests/compute_budget.rs) against the
real BPF programs. Print it with:

```bash
cargo test -p helix-integration-tests --test compute_budget -- --nocapture
```

| Instruction | CU | % of the 200k default |
|---|---:|---:|
| `staking::stake` | 24,261 | 12.1% |
| `staking::claim` | 30,763 | 15.4% |
| `staking::fund_rewards` | 23,436 | 11.7% |
| `staking::set_reward_rate` | 16,729 | 8.4% |
| `governance::create_proposal` | 15,255 | 7.6% |
| `governance::activate_proposal` | 8,695 | 4.3% |
| `governance::cast_vote` | 15,142 | 7.6% |
| `governance::finalize_proposal` | 8,142 | 4.1% |
| `governance::queue_proposal` | 7,866 | 3.9% |
| `governance::execute_treasury_transfer` | 35,883 | 17.9% |
| `treasury::deposit` | 13,959 | 7.0% |

Reproducible byte for byte, but specific to this deployment: PDA bumps derive from the mint
address, and each extra derivation attempt costs 1,500 CU (see below). Read these as
"±1,500 per instruction on a different mint", not as universal constants.

The worst case is `execute_treasury_transfer` at 17.9%, which is the deepest call stack in
the system: governance verifies the proposal, signs as the executor PDA, CPIs into the
treasury, which CPIs into Token-2022. Everything has better than 4× headroom against the
default per-instruction budget, so no caller has to prepend a `ComputeBudget` instruction —
and the test asserts that headroom rather than merely asserting it fits. A program sitting
just inside the budget breaks on the next Anchor or Token-2022 release that costs a few
thousand units more.

### §6.3, measured

| | 1 staker | 4 | 16 | 64 |
|---|---:|---:|---:|---:|
| `stake` | 24,261 | 24,261 | 24,261 | 24,261 |
| `unstake` | 24,289 | 24,289 | 24,289 | 24,289 |
| `claim` | 30,697 | 30,939 | 30,940 | 30,941 |
| `cast_vote` (by prior votes) | 15,019 | 15,019 | 15,019 | 15,019 |

`stake`, `unstake` and `cast_vote` are bit-identical at every count. `claim` moves 244 CU
across the sweep — 0.8% — and the cause is not the staker set: the reward accumulator runs
in `u128`, which SBF has no native instruction for, so LLVM emits software routines whose
cost tracks operand bit-length. Sixty-four times the stake is six more bits.

The controlled experiment separates value from count. Reaching the same `total_weighted`
two different ways — 64 stakers holding one unit each, or one staker holding 64 — costs
**bit-identical 30,941 CU**. Staker count differs by 64× between them and compute does not
move at all, while changing the value alone moves it 244 CU. Count is not a variable;
magnitude is. That is the whole of §6.3.

### Three confounds that had to be controlled first

The first two are larger than the effect being measured, and none is obvious.

**PDA bump search costs 1,500 CU per attempt.** Anchor derives a bump on chain wherever the
constraint is a bare `bump` rather than `bump = <stored>`, which compiles to
`find_program_address` — try 255, then 254, until the candidate is off-curve. Which bump a
given seed set lands on is effectively random, so two otherwise identical stakers can
differ by thousands of units. Measured directly by
`pda_bump_search_costs_more_than_pool_size_ever_does`: bump 251 costs exactly 6,000 CU more
than bump 255. The benchmarks grind their probe keypairs onto the canonical bump so the
comparison is exact rather than merely tolerant — **a tolerance wide enough to absorb the
noise would have been wide enough to hide the growth being looked for.**

**Every PDA descends from the mint, and so does every bump.** `System::bootstrap` generates
a random mint, which moved the whole table by multiples of 1,500 between runs and made
cross-`System` comparison meaningless. An early draft of the magnitude test read that noise
as signal and "confirmed" a mechanism that was not there — it flipped sign on the next run.
The benchmarks now pin the mint, and the table above is reproducible byte for byte.

**A 31-CU quantum of runtime noise remains, and is documented rather than explained away.**
A measurement occasionally lands exactly 31 CU high — never more, never low, most often on
the first measurement after a clock warp. It is not the code under test: twenty stakes by
twenty different owners in one pool measured 24,261 CU each to the unit, and every piece of
protocol state was verified identical across a pair that differed (`total_weighted`,
`reward_per_token` before and after, `last_update_ts`, the settled amount). Past that it is
unattributed, and the tests carry a 64 CU floor to absorb it. It is 0.2% of the smallest
instruction here, against an effect that would be thousands of units — so it bounds the
claim without weakening it.

The general lesson from all three: a benchmark that has not identified its confounds is not
measuring what its name says. Two of these were found only because an assertion was written
tightly enough to fail on them.

## What is NOT covered

Read this section before trusting anything above.

| Gap | Consequence | Fix |
|---|---|---|
| Deployment-time front-running (§5.8) | F-1's *mitigation* is measured and tested; the window before bootstrap lands is not closeable in-program without a deployer gate | Phase 3 |
| Multi-staker distribution at scale | Two or three positions are exercised, not hundreds | Phase 6 (fuzzing) |
| Real-cluster behaviour | LiteSVM is faithful but not a validator; no test covers fees, congestion or reorgs | Phase 3 (devnet) |
| **A browser wallet** | Every dashboard flow is tested against hand-written vectors and simulated by a node executing a message nobody signed. No transaction has been through a wallet extension: an adapter that re-serialises a v0 message, a wallet refusing `replaceRecentBlockhash`, a popup dismissed mid-flow | Needs a person with a wallet, not another test |

### What integration testing found that unit testing could not

Two findings, both structural rather than arithmetic:

**Vesting was unreachable on chain.** `create_stream` requires the governance executor's
signature, and `ProposalAction` had no variant producing it — so no transaction could
create a stream. Nine unit tests covered arithmetic no caller could invoke. Every unit
test passed, the code compiled, and the CPI wiring was correct; the gap was in what
governance is *able to ask for*.
[F-8](./SECURITY-ASSESSMENT.md#f-8--governance-gated-treasury-instructions-are-unreachable),
now fixed and covered by `vesting_e2e.rs`.

**The token-manager admin could not be handed to governance.** `accept_admin` needs the
incoming admin to sign, and the executor PDA signs only inside an `execute_*`, of which
none covered it. Same shape as the previous finding, in a different program.
[F-9](./SECURITY-ASSESSMENT.md#f-9--token-manager-admin-cannot-be-handed-to-governance).

**The reward solvency guard could never approve any rate.** Both halves were individually
tested and individually correct; the defect was in their composition.
[F-2](./SECURITY-ASSESSMENT.md#f-2--reward-liability-computed-from-deposits).

The pattern is the same in all three: the bug lived in the space *between* correct units.
Fixing F-9 nearly produced a fourth instance — granting the realm the admin role without
the admin's powers — which is why the fix covers the whole surface rather than the one
instruction the finding named.

## Integration tests

```bash
anchor build            # required — the tests load target/deploy/*.so
cargo test --workspace
```

Runtime tests live in [`tests/integration/`](../tests/integration) and run against
**LiteSVM**: the real BPF programs and the real Token-2022 program, in-process, in
milliseconds. `TestEnv` provides mint creation with extensions, token accounts, clock
warping, and PDA derivations mirroring each program's seeds.

Staking tests are parameterised over `[plain_mint, fee_bearing_mint]` — as a parameter
rather than a separate suite, because a separate suite drifts.

### Mutation testing, and why it is the real check

A passing test proves nothing until you know it would fail on the bug it claims to catch.
The transfer-fee tests were verified by reverting the fix — crediting `amount` instead of
the vault delta — rebuilding, and confirming they go red:

```text
fee_bearing_mint_credits_the_delta_not_the_argument  FAILED
  position credited 1000000 but vault received 990000
fee_bearing_mint_preserves_vault_solvency            FAILED
  sum of positions (3000000) does not match vault (2970000)
weighted_amount_is_derived_from_the_credited_amount  FAILED
plain_mint_credits_the_full_amount                   ok      <-- still passes
```

The last line is the important one. Under the same mutation the plain-mint test stays
green, which is the entire argument for why unit tests could never have caught this class
of bug. Worth repeating for any invariant that matters: **inject the failure and watch the
test catch it.**

The indexer's attribution was verified the same way. Folding events into the *outermost*
program instead of the innermost — `stack.first()` for `stack.last()`, one word — turns
5 unit tests and 2 reconciliation tests red:

```text
a_nested_cpi_attributes_each_event_to_its_own_program            FAILED
the_indexed_proposal_matches_the_chain_through_execution         FAILED
  the treasury spend was attributed to the wrong program, or lost
  left: 0   right: 2500000

the_indexed_pool_matches_the_chain_through_a_full_staking_lifecycle  ok  <-- still passes
indexed_tvl_follows_credited_amounts_on_a_fee_bearing_mint           ok  <-- still passes
replaying_a_transaction_double_counts_nothing                        ok  <-- still passes
```

Same shape again: the three that stay green are single-program flows, where the innermost
program *is* the outermost and the two rules are indistinguishable. That is the whole
argument for testing the nested case separately.

`close_position` was checked the same way, and this one is worth showing because the
mutation is the implementation most people would write. Decrementing `pool.position_count`
on close — the counter reads like "positions currently open", so treating a close as a
decrement looks obviously right:

```text
closing_does_not_free_the_position_id_for_reuse  FAILED
  close decremented the electorate boundary
  left: 0   right: 1
```

Then, with that first assertion disabled to see whether the second half stands on its own:

```text
closing_does_not_free_the_position_id_for_reuse  FAILED
  called `unwrap_err()` on an `Ok` value
```

That `Ok` is the finding. Re-staking at the closed position's id **succeeded** — the
address was reoccupied, and the new position sits beneath a `position_count_snapshot` taken
before it existed, which is [F-10](./SECURITY-ASSESSMENT.md#f-10--post-snapshot-weight-could-vote)
reopened by a change about reclaiming rent. Two assertions, each catching it independently,
which is what you want for a property this indirect.

### The live tests, and the three that were not testing their claims

The RPC source was written against four documented hazards, with a test named for each.
All six live tests passed on the first run. Then each hazard was injected, and **three of
the four went undetected**:

```text
mutation                        first written        after rework
--------------------------------------------------------------
reverse() removed               NOT CAUGHT           CAUGHT
`err` filter removed            NOT CAUGHT           CAUGHT
signature dedup removed         CAUGHT               CAUGHT
`until` sig instead of slot     NOT CAUGHT           still not caught — see below
```

Each survivor was a different way of being vacuous, and none was visible by reading:

- **Ledger order.** `fetch` sorts the merged batch by slot, so the reversal only changes
  the order of transactions *sharing* a slot — and a test that confirms each transaction
  before sending the next never produces two. It now bursts six treasury deposits on one
  blockhash and asserts that at least two really did land together, because otherwise the
  test proves nothing and says so.
- **Failed transactions.** The original sent a stake that failed on its only instruction,
  which never reaches the `emit!` — so the log carried no event and there was nothing for
  the missing filter to fold. It now sends two instructions where the first succeeds and
  emits, and asserts the failed transaction's log really does contain a `Program data:`
  line before concluding anything from its absence downstream.
- **Cursor resumption.** The original polled until nothing new arrived and stopped. The
  cursor only advances as slots *finalise*, which lags confirmation by tens of slots, so
  it had never left its default and the source was never given a cursor at all. It now
  waits for real finality, with a deadline.

**The fourth mutation is not caught because it is not a bug**, which took measuring the
node to establish rather than reasoning about it. The concern was that `until` names a
signature three of the four followed addresses have never seen. Against Agave 3.1.10:

```text
getSignaturesForAddress(governance, until=<a staking-only signature>)
  -> 0 results; governance had 2 transactions below that slot, none returned
     => `until` is resolved to the signature's slot GLOBALLY, not within the address

getSignaturesForAddress(treasury, until=<earliest of 6 in one slot>)
  -> 5 of 5 same-slot siblings returned
     => and it is exclusive at signature granularity when it can be
```

So both forms are correct here. The slot form is kept for two reasons that need no
measurement — `until` requires the node to still hold that signature in a
transaction-status index that is pruned, and the slot form is what makes resuming
mid-slot expressible — but the earlier claim that the signature form was *wrong* was
itself wrong, and is corrected in [`rpc.rs`](../indexer/src/rpc.rs). **A mutation that
survives is a result, not a gap to be papered over with a test that manufactures a
failure.**

### The descent, mutated before believing it

Phase 4.3 adds two live tests, and after the section above they were mutation-tested before
being trusted. Both hazards are in `fetch_before`, and both produce a projection that is
merely incomplete rather than one that errors:

```text
mutation                                    caught by
-----------------------------------------------------------------------------
page handed back newest-first               a_descent_sees_the_same_history…
next page's bound taken before truncation   a_descent_sees_the_same_history…
                                            a_descent_resumes_from_its_persisted_position
```

The first is worth recording for *how* it was caught. The projection comparison did **not**
fail — reversing a page changes the order events are folded in, and the projection assigns
running totals, so a small reversed page reaches the same state. What failed was the
separate assertion that the descent's slots ascend. That is the same property the live
tests learned the hard way in the section above, arriving from the other direction: an
ordering bug in a projection that assigns is invisible in the projection, so it has to be
asserted about the *sequence* and not about the result.

The second is caught twice, which is the right number for a paging bug: the history comes
back short, and a resumed descent no longer rejoins where it stopped.

### The storage tests, mutated the same way

Same procedure, applied to Phase 4.2 before believing any of it. Ten tests, all passing on
the first run — which after the section above is not evidence of anything. Seven hazards
injected, each the implementation someone would plausibly write:

```text
mutation                                     outcome
-------------------------------------------------------------------------------
applied set not restored on load             CAUGHT  a_redelivery_after_a_restart_does_not_double_count
upsert slot guard removed                    CAUGHT  an_older_slot_cannot_overwrite_a_newer_row
cursor written outside the transaction       CAUGHT  a_failed_commit_leaves_the_cursor_and_the_rows_where_they_were
closed position's row not deleted            CAUGHT  a_closed_position_leaves_no_row_but_keeps_the_electorate_boundary
amount narrowed to i64 on the way in         CAUGHT  an_amount_above_the_signed_range_survives_the_database
`ON CONFLICT DO NOTHING` dropped on events   CAUGHT  committing_the_same_batch_twice_changes_nothing
unfinalised transactions reported as settled CAUGHT  only_finalised_transactions_are_written
```

Every one was caught by exactly the test named for it and by no other, which is the
property worth having: a suite where one mutation reddens nine tests is telling you the
tests overlap, not that they are thorough.

Two of the seven only work because the test constructs the situation deliberately, and both
say so in an assertion rather than in a comment:

- **The redelivery test first proves the hazard exists.** `Staked` accumulates, unlike most
  of the projection, and the test asserts that folding it under two signatures really does
  double the total before asserting that folding it under one does not. Without that, a
  future change making `Staked` carry a running total would silently turn the test into a
  tautology.
- **The atomicity test drops the `votes` table** so a statement fails *after* the event rows
  and several projection tables have been written. A failure on the first statement would
  prove nothing about a torn write.

An eighth mutation targets the coverage check rather than the store: putting
`RealmParamsUpdated` back the way it was — absent from the decoder's list — is CAUGHT by
`the_indexer_decodes_every_event_the_programs_declare`. Reproducing it needs edits in three
files, because the fold and the store are exhaustive matches and so are compile-coupled to
the list. **Only the list itself is unforced, which is exactly why it was the thing that
drifted.**

**One procedural note, learned the hard way here.** A mutation script that restores files
with `shutil.move` restores their *mtime* too, so cargo sees an unchanged source and reuses
the mutated binary. The next full run reported a failure that no longer existed in the
source. This is the same hazard as "`cargo test` does not rebuild the programs", one level
up: touch the file after restoring, or verify against a clean rebuild.

### What the operator rehearsal caught that no test did

The runbook step "point `helix-index` at the cluster, then restart it" is not redundant with
the suite. Run against a validator still holding the previous build, it printed:

```text
anomaly: 31PhgSLZ...xpLXZ UndecodableData { log_index: 23, program: Governance }
... 5 times, and 0 realms
```

`RealmInitialized` had gained `min_weight_to_propose` in this phase, so the deployed program
was emitting the old, shorter body and Borsh refused it. Every test passed — because the
tests build the event and decode it with the *same* build, which is precisely the pair that
can never disagree. Only two different builds, one on chain and one in the reader, can
demonstrate this, and only a deployment produces that pair.

The behaviour is correct: refusing beats decoding a prefix and defaulting the rest. It is
also the mirror of the case `event.rs` documents, where an anomaly means the indexer is
*older* than the chain. **The rehearsal is the only thing in this repository that can tell
you which of the two you have**, which is why it is a runbook step rather than a suggestion.

## CI

[`.github/workflows/ci.yml`](../.github/workflows/ci.yml) runs on every push and PR:

1. **lint** — `fmt --check`, `clippy -D warnings`, eslint/prettier. Fast, fails first.
2. **audit** — `cargo audit` against RustSec.
3. **test** — `anchor keys verify`, `anchor build` with the stack-offset grep, then
   `cargo test --workspace` (unit + doc + runtime, including the compute
   benchmarks). Artifacts (`.so`, IDL) retained 14
   days. `anchor test` is deliberately not run: there is no TypeScript suite, and stubbing
   it green would report a passing integration suite containing nothing.

`RUSTFLAGS: -D warnings` is set workspace-wide, so a warning fails the build. Warnings
that are permitted to accumulate stop being read.

## Adding a test

1. Put it next to the logic, in the same file's `mod tests`. Pure functions are the unit,
   not instructions.
2. Name it as the property, not the mechanism: `liability_is_never_understated`, not
   `test_liability_2`. The name is what a reviewer reads when it fails.
3. If it covers an invariant, cite the section — `INVARIANTS.md §3.3` — and add the test
   name to that table.
4. If it is a regression, say what broke and in which direction. Future readers need to
   know what the test is defending against, and the comment is the only place that lives.
