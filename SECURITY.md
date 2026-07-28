# Security Policy

## Status

Helix is **unaudited software on devnet**. It has not been reviewed by a third-party
audit firm and holds no real value. Do not deploy it to mainnet or put funds at risk
without an independent audit.

## Reporting a vulnerability

Report privately — please do not open a public issue for anything exploitable.

- GitHub private vulnerability reporting (Security → Report a vulnerability), or
- email the address on the repository owner's GitHub profile.

Include the affected program, the conditions required, and a proof-of-concept test
against the local validator if you have one. A failing test in `tests/` is the most
useful possible report.

Expect an acknowledgement within 72 hours and an assessment within seven days.

## Scope

**In scope** — the four programs under [`programs/`](./programs):

- Loss or lock-up of staked principal or accrued rewards
- Minting outside the registered-minter path or epoch caps
- Treasury spends without a passed, timelocked proposal
- Vote weight obtained without a qualifying locked position
- Timelock or proposal-lifecycle bypass
- Arithmetic errors producing incorrect balances or vote tallies
- Denial of service that permanently prevents withdrawal

**Out of scope** — see [docs/THREAT-MODEL.md](./docs/THREAT-MODEL.md#out-of-scope).
Briefly: RPC/frontend compromise, validator-level censorship and MEV, majority stake
capture, and anything requiring a user's own key.

## Security practices in this repository

In place today:

- `overflow-checks = true` in the release profile, plus explicit `checked_*` at every
  arithmetic site — the profile is a backstop, not the design
- Documented invariants, each marked with whether a test actually asserts it:
  [docs/INVARIANTS.md](./docs/INVARIANTS.md)
- A structured self-review with an access-control matrix and risk register:
  [docs/SECURITY-ASSESSMENT.md](./docs/SECURITY-ASSESSMENT.md)
- CI runs `cargo clippy -D warnings`, `cargo audit`, and the unit suite on every push,
  and greps the build log for SBF stack-frame overflows — which `anchor build` reports as
  errors while still exiting 0

Planned, and **not** yet in place — see [docs/ROADMAP.md](./docs/ROADMAP.md):

- Integration tests covering cross-program flows and fee-bearing Token-2022 mints
  (Phase 2) — the largest current gap
- Stateful fuzzing via Trident, with the invariant set as the oracle (Phase 6)
- Verifiable builds via `solana-verify`, so deployed bytecode can be reproduced from this
  source tree (Phase 3)
- Upgrade authority held by a multisig, then transferred to governance (Phases 3 and 7)

The distinction matters: an unaudited protocol that overstates its controls is more
dangerous than one that states them plainly.
