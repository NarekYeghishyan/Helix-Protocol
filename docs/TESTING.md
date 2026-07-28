# Testing procedures

*Deliverable 5 — testing procedures.*

This document is deliberately explicit about what is **not** covered. A test suite's value
is bounded by the honesty of its coverage claims.

## Running the suite

```bash
anchor build                                                  # required first — the
                                                              # runtime tests load .so files
cargo test --workspace                                        # 97 tests: unit + doc + runtime
cargo test -p helix-staking --lib                             # one program's unit tests
cargo test -p helix-staking --lib -- --nocapture rounding     # one test, with output

cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check

anchor build 2>&1 | tee build.log
grep -i "stack offset" build.log     # MUST be empty — see below
```

### The one non-obvious step

`anchor build` reports SBF stack-frame overflows as `Error:` and **exits 0 anyway**. A
program that overflows its 4KB frame may corrupt memory at runtime, so the exit code is
actively misleading. Always grep the log; CI does the same rather than trusting the
status code. See [F-3](./SECURITY-ASSESSMENT.md#f-3--sbf-stack-frame-overflow).

## What is covered

**65 unit tests** over pure functions and state machines, and **32 runtime tests** against
the real BPF programs.

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

### Runtime tests

| File | Tests | What is asserted |
|---|---|---|
| `smoke.rs` | 2 | All four programs load and are executable; IDs are distinct |
| `staking_transfer_fee.rs` | 4 | §1.1, §1.3, §2.1–§2.3 against a real 1% transfer-fee mint |
| `staking_lifecycle.rs` | 12 | §1.2, §1.4, §6.1, §6.2, §6.4, §6.5 — funding, rate solvency, accrual, claim, partial and full unstake, pause semantics, cross-owner claim refused |
| `governance_e2e.rs` | 14 | §4.1–4.7, §4.11–4.12, §5.1 — the authority chain plus one negative test per threat-model attack |

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

**Test the predicate the handler evaluates, not just its inputs.** This is the lesson from
[F-2](./SECURITY-ASSESSMENT.md#f-2--reward-liability-computed-from-deposits): both halves
of the solvency guard were individually correct and individually tested, and the defect
lived in their composition — a guard that could never approve any non-zero reward rate.
`solvency_ok()` in the staking tests now mirrors the handler's actual comparison.

## What is NOT covered

Read this section before trusting anything above.

| Gap | Consequence | Fix |
|---|---|---|
| **No vesting runtime tests** | Blocked: vesting is currently *unreachable on chain* — see below | [ROADMAP](./ROADMAP.md) 2.6 then 2.7 |
| No compute benchmarks | Invariant §6.3 is argued from code structure, not measured | Phase 6 |
| No fuzzing | Only hand-chosen inputs have been tried | Phase 6 |
| No deployment-time test for §5.8 | Initialiser front-running (F-1) is mitigated operationally, not tested | Phase 3 |
| Multi-staker reward distribution at scale | Two positions are exercised, not hundreds | Phase 6 (fuzzing) |

### The vesting gap is not just missing tests

Writing the vesting runtime test found that **there is no way to construct a transaction
that creates a stream**. `create_stream`, `revoke_stream`, `set_spend_cap` and
`set_governance_executor` all require the governance executor's signature, and
`ProposalAction` has no variant that produces it for them — only `spend` is reachable.

Every unit test passed. The code compiled. The CPI wiring was correct. The gap was in what
governance is *able to ask for*, which is not a property any unit test observes. Recorded
as [F-8](./SECURITY-ASSESSMENT.md#f-8--governance-gated-treasury-instructions-are-unreachable);
the fix precedes the tests.

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

## CI

[`.github/workflows/ci.yml`](../.github/workflows/ci.yml) runs on every push and PR:

1. **lint** — `fmt --check`, `clippy -D warnings`, eslint/prettier. Fast, fails first.
2. **audit** — `cargo audit` against RustSec.
3. **test** — `anchor keys verify`, `anchor build` with the stack-offset grep, then
   `cargo test --workspace` (unit + doc + runtime). Artifacts (`.so`, IDL) retained 14
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
