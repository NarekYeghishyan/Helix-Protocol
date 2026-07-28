# treasury

Holds protocol funds. There is no address anywhere that can move them directly.

The vault authority is a PDA, and spending requires a signature from the
`governance_executor` — a PDA only the governance program can produce, and only
while executing a proposal that passed quorum *and* cleared its timelock. That chain
is the security model.

## Two limits on top of the governance signature

**Per-epoch spend cap.** Even a genuinely passed malicious proposal cannot empty the
treasury in one transaction. This does not prevent a determined majority attacker — see
`docs/THREAT-MODEL.md` A11 — but it caps the blast radius per epoch and buys time for
the guardian veto and for holders to exit.

**Committed balance.** `committed_to_streams` tracks the unclaimed remainder of every
live vesting stream, and `spend` can only touch the balance above it. Without this, a
spend could pay out tokens already promised to a beneficiary, who would discover it
only when their claim failed (`INVARIANTS.md` §1.6).

Streams are *not* escrowed into per-beneficiary accounts. Escrow would work, but costs
a token account per beneficiary and makes the treasury's real balance harder to read;
the commitment counter gives the same guarantee.

## Vesting

Linear release with an optional cliff. Vesting accrues from `start_ts`, and the cliff
gates *when* the first claim is possible — so at the cliff everything accrued since
start is released at once. The standard "4-year schedule, 1-year cliff" shape.

Revocation is **forward-only**. `revoked_at` freezes evaluation at that instant:

- Already-vested tokens stay claimable by the beneficiary.
- Only the unvested remainder returns to the treasury's spendable balance.
- Revoking twice is refused, so the freeze timestamp cannot be moved later to reduce
  what the beneficiary has already earned.

The alternatives are both wrong: evaluating at `now` after revocation keeps accruing
for someone who was cut off, and treating the whole stream as void confiscates tokens
the beneficiary earned. Freezing is the only option that does neither.

Truncation runs in the treasury's favour, so vesting is marginally slow rather than
marginally fast — but the endpoint releases the full amount, so the beneficiary
receives exactly `total_amount` in the end, never less.

## Instructions

| Instruction | Authority | Notes |
|-------------|-----------|-------|
| `initialize_treasury` | anyone (once per mint) | Names the governance executor |
| `deposit` | anyone | Credits the observed vault delta |
| `spend` | governance executor | Epoch cap + uncommitted balance |
| `create_stream` | governance executor | Must be backed by uncommitted balance |
| `claim_stream` | beneficiary | Works on revoked streams too |
| `revoke_stream` | governance executor | Forward-only |
| `set_spend_cap` | governance executor | |
| `set_governance_executor` | current executor | No admin escape hatch |

`set_governance_executor` is callable only by the current executor, and there is
deliberately no admin override — one would make every other guarantee here conditional on
whoever holds it.

> **Known gap (F-8).** `create_stream`, `revoke_stream`, `set_spend_cap` and
> `set_governance_executor` all require the executor's signature, but `helix-governance`
> currently has no `ProposalAction` variant that produces it for them — only `spend` is
> reachable. So vesting is presently unreachable on chain, the spend cap is immutable
> after initialisation, and governance migration is not yet possible. Found by attempting
> to write the vesting runtime test; see
> [SECURITY-ASSESSMENT.md F-8](../../docs/SECURITY-ASSESSMENT.md#f-8--governance-gated-treasury-instructions-are-unreachable).

## Tests

`cargo test -p helix-treasury` — 17 unit tests over the spend-budget state machine
(including that ten idle epochs do not grant ten epochs of allowance) and the vesting
schedule (cliff behaviour, truncation direction, and the three revocation properties
above).
