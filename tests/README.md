# Integration tests

**Not implemented yet.** This directory exists so the gap is visible rather than implied
by an empty path in the docs.

Integration tests are [Phase 2](../docs/ROADMAP.md#phase-2--integration-tests-46-days-high-confidence)
and the highest-priority work in the project. Everything else waits on them, because the
cross-program flows currently type-check but have never been executed:

- the governance executor PDA signing a treasury spend
- the lock gate rejecting a real staking position
- **a fee-bearing Token-2022 mint crediting the observed vault delta**

The last one is the sharpest gap. On a plain SPL mint the vault delta and the `amount`
argument are identical, so every existing test passes whether the code credits one or the
other — the correct behaviour could be deleted and nothing would go red. See
[INVARIANTS.md §2](../docs/INVARIANTS.md#2-token-2022-transfer-fees).

Planned design, including the LiteSVM/Surfpool split and the requirement that every
staking test is parameterised over `[plain_mint, fee_bearing_mint]`, is in
[TESTING.md](../docs/TESTING.md#planned-integration-test-design).

Until this directory has tests in it, `anchor test` is not wired up, and CI runs the unit
suite and the BPF build only. What is covered today:

```bash
cargo test --workspace --lib    # 64 unit tests
```
