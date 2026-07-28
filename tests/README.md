# Integration tests

Runtime tests executed against [LiteSVM](https://github.com/LiteSVM/litesvm) — the real
BPF programs, the real Token-2022 program, in-process and in milliseconds.

```bash
anchor build          # required: the tests load target/deploy/*.so
cargo test --workspace
```

| File | Covers |
|---|---|
| [`integration/tests/smoke.rs`](./integration/tests/smoke.rs) | All four programs load and are executable; program IDs are distinct |
| [`integration/tests/staking_transfer_fee.rs`](./integration/tests/staking_transfer_fee.rs) | Invariants §1.1, §1.3, §2.1–§2.3 — Token-2022 transfer fees |
| [`integration/tests/governance_e2e.rs`](./integration/tests/governance_e2e.rs) | §4.1–4.7, §4.11–4.12, §5.1 — the authority chain, plus a negative test per threat-model attack |
| [`integration/src/lib.rs`](./integration/src/lib.rs) | Harness: mint creation with extensions, clock warping, PDA derivations |
| [`integration/src/bootstrap.rs`](./integration/src/bootstrap.rs) | A fully wired system — pool, realm, treasury, authorities connected |

## Why the transfer-fee tests matter most

Staking credits the **observed vault balance delta**, never the `amount` argument. On a
plain SPL mint those two numbers are identical — so the entire 65-test unit suite passes
whether the code does the right thing or the wrong thing.

That was confirmed by mutation testing. Reverting the fix so deposits credit `amount`:

```text
fee_bearing_mint_credits_the_delta_not_the_argument  FAILED
  position credited 1000000 but vault received 990000
fee_bearing_mint_preserves_vault_solvency            FAILED
  sum of positions (3000000) does not match vault (2970000)
weighted_amount_is_derived_from_the_credited_amount  FAILED
plain_mint_credits_the_full_amount                   ok      <-- still passes
```

A 30,000-unit shortfall between what the pool believes it owes and what it holds. The
plain-mint test staying green under the same mutation is the whole argument for why this
suite had to exist.

## Harness notes

**`litesvm` is pinned to `=0.13.1`.** Newer versions track a newer Solana than
anchor-lang 1.1.2, which puts two `wincode` versions in the graph and fails with an error
that reads like a litesvm bug rather than a version mismatch. See the comment in
[`integration/Cargo.toml`](./integration/Cargo.toml).

**Token-2022 instructions are built through `anchor_spl::token_2022`'s re-export** of
`spl_token_2022_interface`, so the test harness and the programs are always compiled
against the same token version.

**Extension state must be initialised before `initialize_mint2`**, and the mint account
sized for its extensions up front — Token-2022 will not grow it later. `TestEnv::create_mint`
handles both.

## Negative tests must name their expected failure

Every negative test asserts a **specific** error, not merely that something failed:

```rust
let err = sys.env.try_send(&[ix], &[]).expect_err("must fail");
assert!(err.contains("TimelockNotElapsed"), "unexpected failure: {err}");
```

An earlier draft also accepted any error containing `0x`, which is nearly every program
error. That permissiveness hid a test passing for the wrong reason: the guardian had no
lamports, so its vote failed on rent for the vote record rather than on authorisation.
Tightening the assertion exposed it immediately.

`try_send` returns the failure **with program logs attached**, because an Anchor code on
its own (`custom program error: 0x1771`) says nothing while the log line names the
constraint.

## Still missing

Staking's withdrawal lifecycle (accrue → claim → unlock → unstake) and vesting token
movement. Tracked as Phase 2.2b and 2.6 in [ROADMAP.md](../docs/ROADMAP.md).
