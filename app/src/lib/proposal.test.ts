/**
 * The action form, and the parsing that stands between a text box and a vote.
 *
 * Two different things are being checked here and they need different kinds of
 * assertion.
 *
 * The **form derivation** is checked against a hand-written list of the
 * `ProposalAction` variants, transcribed from `state.rs`. Comparing it to the IDL
 * would be circular — the derivation reads the IDL — and the list is the point:
 * `governance/README.md` claims the set of things governance can do is closed and
 * visible, so a variant appearing or disappearing should require someone to look.
 *
 * The **parsing** is checked against the values that break it. Every one of these
 * ends up in a proposal people vote on and a program later executes, so the
 * interesting cases are all the ones where a lenient parser would produce a
 * plausible wrong number instead of an error.
 */

import { strict as assert } from "node:assert";
import { test } from "node:test";

import { PublicKey } from "@solana/web3.js";

import { encodeType } from "./coder.ts";
import {
  composeAction,
  describeSeconds,
  parseField,
  proposalActions,
  type ActionField,
} from "./proposal.ts";
import { GOVERNANCE } from "./programs.ts";

const variants = () => proposalActions();
const variant = (name: string) => {
  const found = variants().find((v) => v.name === name);
  assert.ok(found, `no variant ${name}`);
  return found;
};

// --------------------------------------------------------------- the closed set

test("the action set is the closed one the program declares", () => {
  // Transcribed from `ProposalAction` in programs/governance/src/state.rs. The
  // enum is closed on purpose — a voter reads the variant and knows the blast
  // radius — so this list changing is a governance-surface change and should
  // require a human to notice, not just a re-run.
  assert.deepEqual(
    variants().map((v) => v.name),
    [
      "Signal",
      "TreasuryTransfer",
      "SetStakingRewardRate",
      "CreateVestingStream",
      "RevokeVestingStream",
      "SetTreasurySpendCap",
      "SetGovernanceExecutor",
      "AcceptTokenManagerAdmin",
      "RegisterMinter",
      "UpdateMinter",
      "RevokeMinter",
      "SetTokenPaused",
      "ProposeTokenAdmin",
      "UpdateRealmParams",
      "SetRealmAuthority",
    ],
  );
});

test("a unit variant asks for nothing", () => {
  for (const name of ["Signal", "AcceptTokenManagerAdmin", "RevokeMinter"]) {
    assert.deepEqual(variant(name).fields, [], `${name} grew a field`);
  }
});

test("field types decide the input, and nothing else does", () => {
  // Not the field *name*. `epoch_duration` and `end_ts` are both `i64` and mean
  // different things; a form keyed on names would guess, and would be silently
  // wrong for whichever it guessed against.
  assert.deepEqual(
    variant("CreateVestingStream").fields.map((f) => [f.path, f.type, f.input]),
    [
      ["beneficiary", "pubkey", "address"],
      ["total_amount", "u64", "amount"],
      ["start_ts", "i64", "seconds"],
      ["cliff_ts", "i64", "seconds"],
      ["end_ts", "i64", "seconds"],
    ],
  );

  assert.deepEqual(
    variant("SetTokenPaused").fields.map((f) => [f.path, f.input]),
    [["paused", "boolean"]],
  );
});

test("a nested struct is flattened onto dotted paths", () => {
  // `UpdateRealmParams` carries a `RealmParams`, the only nesting these programs
  // have. A form that could not reach inside it would offer a variant with no
  // inputs — which would encode a proposal setting every parameter to zero.
  assert.deepEqual(
    variant("UpdateRealmParams").fields.map((f) => [f.path, f.type]),
    [
      ["params.quorum_bps", "u16"],
      ["params.approval_bps", "u16"],
      ["params.voting_period", "i64"],
      ["params.timelock_delay", "i64"],
      ["params.min_weight_to_propose", "u64"],
    ],
  );
});

// ------------------------------------------------------------------- parsing

const field = (path: string, type: ActionField["type"], input: ActionField["input"]) =>
  ({ path, name: path.split(".").pop()!, type, input }) as ActionField;

test("a blank field is refused rather than becoming zero", () => {
  // `BigInt("")` is `0n`, which is the trap `amount.ts` exists for — and in this
  // path it would turn an unfilled box into a proposal that transfers nothing
  // while claiming to transfer something.
  assert.throws(
    () => parseField(field("amount", "u64", "amount"), ""),
    /amount: is required/,
  );
  assert.throws(() => parseField(field("amount", "u64", "amount"), "   "), /is required/);
});

test("a decimal amount is refused, not truncated", () => {
  // These are base units. Accepting "1.5" and flooring it would send a different
  // amount than the proposal text says.
  assert.throws(() => parseField(field("amount", "u64", "amount"), "1.5"), /whole number/);
  assert.throws(() => parseField(field("amount", "u64", "amount"), "1e9"), /whole number/);
});

test("wide integers parse to bigint and narrow ones to number", () => {
  // The coder refuses the wrong one on purpose, so this split has to match it.
  assert.equal(parseField(field("amount", "u64", "amount"), "9007199254740993"), 9_007_199_254_740_993n);
  assert.equal(parseField(field("quorum_bps", "u16", "number"), "1000"), 1000);
  assert.equal(parseField(field("start_ts", "i64", "seconds"), "-1"), -1n);
});

test("an amount past the double limit survives the form", () => {
  // End to end: text box to encoded bytes, without passing through a `number`.
  const action = composeAction(variant("TreasuryTransfer"), {
    destination: PublicKey.default.toBase58(),
    amount: "18446744073709551615",
  });
  const bytes = encodeType(GOVERNANCE, { defined: { name: "ProposalAction" } }, action);

  // 1 tag + 32 pubkey + 8 amount, and the amount is u64::MAX.
  assert.equal(bytes.length, 41);
  assert.deepEqual([...bytes.subarray(33)], [255, 255, 255, 255, 255, 255, 255, 255]);
});

test("an address is checked for being one, not just for decoding", () => {
  const real = PublicKey.unique().toBase58();
  assert.ok((parseField(field("destination", "pubkey", "address"), real) as PublicKey).equals(new PublicKey(real)));

  assert.throws(
    () => parseField(field("destination", "pubkey", "address"), "not-an-address"),
    /not a valid base58 address/,
  );
  // Leading zeros are a different string that decodes to the same key, and
  // accepting one would let a proposal display an address nobody can search for.
  assert.throws(
    () => parseField(field("destination", "pubkey", "address"), `1${real}`),
    /base58 address/,
  );
});

test("a checkbox is a boolean and a boolean field is not text", () => {
  assert.equal(parseField(field("paused", "bool", "boolean"), true), true);
  assert.equal(parseField(field("enabled", "bool", "boolean"), false), false);
  assert.throws(() => parseField(field("paused", "bool", "boolean"), "true"), /checkbox/);
});

// ------------------------------------------------------------------ composing

test("a composed action encodes as the variant the program expects", () => {
  const destination = PublicKey.unique();
  const action = composeAction(variant("TreasuryTransfer"), {
    destination: destination.toBase58(),
    amount: "250000",
  });

  assert.deepEqual(action, { kind: "TreasuryTransfer", destination, amount: 250_000n });

  // The tag is the variant's index — second of fifteen.
  const bytes = encodeType(GOVERNANCE, { defined: { name: "ProposalAction" } }, action);
  assert.equal(bytes[0], 1);
});

test("a nested action is rebuilt into the shape the coder wants", () => {
  const action = composeAction(variant("UpdateRealmParams"), {
    "params.quorum_bps": "1000",
    "params.approval_bps": "6000",
    "params.voting_period": "259200",
    "params.timelock_delay": "86400",
    "params.min_weight_to_propose": "1000000",
  });

  assert.deepEqual(action, {
    kind: "UpdateRealmParams",
    params: {
      quorum_bps: 1000,
      approval_bps: 6000,
      voting_period: 259_200n,
      timelock_delay: 86_400n,
      min_weight_to_propose: 1_000_000n,
    },
  });

  // And it round-trips: the flattening is undone exactly, not approximately.
  assert.doesNotThrow(() =>
    encodeType(GOVERNANCE, { defined: { name: "ProposalAction" } }, action),
  );
});

test("every variant composes and encodes from plausible input", () => {
  // A sweep, so a variant added to the program without a matching input kind
  // fails here rather than in front of someone drafting a proposal.
  const sample = (f: ActionField): string | boolean => {
    switch (f.input) {
      case "address":
        return PublicKey.unique().toBase58();
      case "boolean":
        return true;
      case "amount":
        return "1000";
      case "seconds":
        return "86400";
      default:
        return "6000";
    }
  };

  for (const v of variants()) {
    const values = Object.fromEntries(v.fields.map((f) => [f.path, sample(f)]));
    const action = composeAction(v, values);
    assert.doesNotThrow(
      () => encodeType(GOVERNANCE, { defined: { name: "ProposalAction" } }, action),
      `${v.name} composed into something the coder refuses`,
    );
  }
});

test("a missing field names itself rather than encoding a default", () => {
  assert.throws(
    () => composeAction(variant("TreasuryTransfer"), { amount: "1" }),
    /destination: is required/,
  );
});

// ------------------------------------------------------------------ display

test("an i64 is described both ways, because the IDL does not say which", () => {
  const description = describeSeconds("86400");
  assert.match(description ?? "", /1d as a duration/);
  assert.match(description ?? "", /1970-01-02T00:00:00Z as a timestamp/);

  // A plausible timestamp gets the same treatment — showing only one reading
  // would be right most of the time and silently wrong the rest.
  const later = describeSeconds("1800000000");
  assert.match(later ?? "", /as a duration/);
  assert.match(later ?? "", /2027-01-15T08:00:00Z/);

  assert.equal(describeSeconds(""), null);
  assert.equal(describeSeconds("abc"), null);
  assert.equal(describeSeconds("0"), null);
});
