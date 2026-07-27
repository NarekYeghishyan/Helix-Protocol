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

- `overflow-checks = true` in the release profile, plus explicit `checked_*` at every
  arithmetic site — the profile is a backstop, not the design
- Documented invariants with tests that assert them: [docs/INVARIANTS.md](./docs/INVARIANTS.md)
- Stateful fuzzing via Trident
- CI runs `cargo clippy -D warnings`, `cargo audit`, and the full test suite on every push
- Verifiable builds via `anchor verify` / `solana-verify`, so the deployed bytecode can
  be reproduced from this source tree
- Upgrade authority held by a multisig during beta, with transfer to governance as the
  documented end state
