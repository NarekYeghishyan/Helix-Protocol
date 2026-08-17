/**
 * Error decoding, checked against the programs' own numbering rules.
 *
 * Every code below is worked out from the `#[error_code]` enums rather than read
 * back out of the IDL, for the same reason the other tests transcribe account
 * lists by hand: a lookup that agrees with the file it came from proves the file
 * was opened.
 *
 * `errors.rs` in the staking program has a comment explaining that variants are
 * appended and never inserted, because Anchor numbers them from 6000 in
 * declaration order. The first test here is that comment turned into something
 * that fails.
 */

import { strict as assert } from "node:assert";
import { test } from "node:test";

import governance from "../idl/helix_governance.ts";
import staking from "../idl/helix_staking.ts";
import { decodeTransactionError, failingProgramFromLogs, programLogs } from "./errors.ts";
import { STAKING_PROGRAM_ID, SYSTEM_PROGRAM_ID } from "./programs.ts";

test("program errors are numbered from 6000 in declaration order", () => {
  for (const idl of [staking, governance]) {
    const errors = idl.errors ?? [];
    assert.ok(errors.length > 0, `${idl.metadata.name} declares no errors`);
    errors.forEach((error, index) => {
      assert.equal(
        error.code,
        6000 + index,
        `${idl.metadata.name}::${error.name} is not where Anchor's numbering puts it`,
      );
    });
  }
});

test("a staking error decodes to the message the program was given", () => {
  // `PositionLocked` is the seventh variant of `StakingError`: NotAuthority,
  // NoPendingAuthority, NotPendingAuthority, DepositsPaused, BelowMinimumStake,
  // ZeroAmount, PositionLocked — so 6000 + 6.
  const decoded = decodeTransactionError({ InstructionError: [0, { Custom: 6006 }] }, [
    `Program ${STAKING_PROGRAM_ID.toBase58()} failed: custom program error: 0x1776`,
  ]);

  assert.equal(decoded.name, "PositionLocked");
  assert.equal(decoded.message, "Position is still locked");
  assert.equal(decoded.program, "helix_staking");
  assert.equal(decoded.instructionIndex, 0);
});

test("the error that exists because the alternative sent people to the wrong place", () => {
  // `UnexpectedPositionId` is the last variant, added when `close_position` gave
  // the id check a second way to be hit. Before it, a lost race against another
  // staker was reported as `MathOverflow`.
  const last = (staking.errors ?? []).at(-1);
  assert.equal(last?.name, "UnexpectedPositionId");

  const decoded = decodeTransactionError({ InstructionError: [0, { Custom: last!.code }] });
  assert.match(decoded.message, /position_count/);
});

test("a code is resolved against the program the logs say failed", () => {
  // 6000 exists in both IDLs and means different things. Attributing it to the
  // top-level program would report the wrong one whenever governance CPIs into
  // the treasury or the staking program.
  const asStaking = decodeTransactionError({ InstructionError: [0, { Custom: 6000 }] }, [
    `Program ${STAKING_PROGRAM_ID.toBase58()} failed: custom program error: 0x1770`,
  ]);
  assert.equal(asStaking.program, "helix_staking");

  const asGovernance = decodeTransactionError({ InstructionError: [0, { Custom: 6000 }] }, [
    `Program ${governance.address} failed: custom program error: 0x1770`,
  ]);
  assert.equal(asGovernance.program, "helix_governance");

  // The two really are different messages, so the attribution matters.
  assert.notEqual(asStaking.message, asGovernance.message);
});

test("a second vote from the same position is named, not left as error 0", () => {
  // Anchor's `init` on an existing account surfaces as system program error 0.
  // The system program has no IDL, so without a case for it this decodes to
  // nothing — and it is the most reachable failure in the whole UI.
  const decoded = decodeTransactionError({ InstructionError: [0, { Custom: 0 }] }, [
    `Program ${SYSTEM_PROGRAM_ID.toBase58()} failed: custom program error: 0x0`,
  ]);

  assert.match(decoded.message, /already exists/);
  assert.equal(decoded.program, "system");
});

test("an unrecognised code keeps its number instead of being explained away", () => {
  const decoded = decodeTransactionError({ InstructionError: [1, { Custom: 9999 }] });
  assert.match(decoded.message, /9999/);
  assert.equal(decoded.code, 9999);
  assert.equal(decoded.instructionIndex, 1);

  // And an Anchor constraint code is placed in its range rather than guessed at.
  const constraint = decodeTransactionError({ InstructionError: [0, { Custom: 2222 }] });
  assert.match(constraint.message, /Anchor constraint error 2222/);
});

test("runtime failures are phrased for someone holding a wallet", () => {
  assert.match(decodeTransactionError("BlockhashNotFound").message, /expired/);
  assert.match(decodeTransactionError("ProgramAccountNotFound").message, /not deployed/);

  // And anything unrecognised is passed through rather than replaced.
  assert.equal(decodeTransactionError("SomethingNew").message, "SomethingNew");
});

test("nothing is swallowed — the raw error survives every path", () => {
  const weird = { SomeNewShape: [1, 2, 3] };
  const decoded = decodeTransactionError(weird);
  assert.equal(decoded.raw, JSON.stringify(weird));
  assert.match(decoded.message, /refused the transaction/);
});

test("the failing program is the last one the logs blamed", () => {
  const logs = [
    `Program ${governance.address} invoke [1]`,
    `Program ${STAKING_PROGRAM_ID.toBase58()} invoke [2]`,
    `Program ${STAKING_PROGRAM_ID.toBase58()} failed: custom program error: 0x1776`,
    `Program ${governance.address} failed: custom program error: 0x1776`,
  ];
  // The outermost failure is logged last, and it is the one the transaction
  // error belongs to.
  assert.equal(failingProgramFromLogs(logs), governance.address);
  assert.equal(failingProgramFromLogs(null), undefined);
  assert.equal(failingProgramFromLogs(["nothing here"]), undefined);
});

test("program logs keep the program's own lines and drop the framework's", () => {
  const logs = [
    `Program ${STAKING_PROGRAM_ID.toBase58()} invoke [1]`,
    "Program log: Instruction: Stake",
    "Program data: c29tZQ==",
    `Program ${STAKING_PROGRAM_ID.toBase58()} consumed 41234 of 200000 compute units`,
    `Program ${STAKING_PROGRAM_ID.toBase58()} success`,
  ];

  assert.deepEqual(programLogs(logs), ["Instruction: Stake", "c29tZQ=="]);
  assert.deepEqual(programLogs(null), []);
});
