/**
 * Turning a failed simulation into something a person can act on.
 *
 * The specific error enums in these programs exist so a UI can say "this
 * position is still locked" instead of "custom program error: 0x1771". That only
 * pays off if something does the lookup, and the raw code is what every wallet
 * and explorer shows by default.
 *
 * Program errors are read out of the IDLs — the same generated artifact the
 * instruction encoder uses — so a renumbered error cannot leave this file
 * confidently reporting the previous meaning. Anchor's *framework* codes are a
 * short table below, because they are not in any program's IDL; the ones listed
 * are the ones a wallet flow can actually provoke, and anything unlisted is
 * reported as an unknown code in a named range rather than guessed at.
 */

import type { Idl } from "./idl.ts";
import { GOVERNANCE, STAKING, SYSTEM_PROGRAM_ID } from "./programs.ts";

/** What went wrong, in the most specific terms available. */
export interface DecodedError {
  /** One line, safe to put in front of a user. */
  message: string;
  /** The program's own error name, when the failure came from one. */
  name?: string;
  code?: number;
  /** Which program failed, when it can be told. */
  program?: string;
  /** Index of the failing instruction within the transaction. */
  instructionIndex?: number;
  /** Anything that could not be interpreted, kept so nothing is swallowed. */
  raw: string;
}

/**
 * Framework errors worth naming.
 *
 * Deliberately short. A long table copied from Anchor's source is a second copy
 * of something that changes on their release schedule, and a *wrong* name is
 * worse than a number — these are the codes a stake/claim/vote flow actually
 * produces, and the fallback below is honest about the rest.
 */
const ANCHOR_ERRORS: Record<number, string> = {
  2001: "A `has_one` constraint failed: an account passed does not belong to the one that owns it.",
  2003: "A constraint on one of the accounts failed.",
  2006: "A seed constraint failed — a derived address does not match the account passed.",
  2012: "An account is not the fixed address the program requires.",
  3003: "An account could not be deserialised — it is not the type this instruction expects.",
  3007: "An account is owned by the wrong program.",
  3012: "An account this instruction needs has not been created yet.",
};

/** Anchor's error-code ranges, used only to say *where* an unknown code is from. */
function rangeOf(code: number): string | undefined {
  if (code >= 100 && code < 1000) return "Anchor instruction error";
  if (code >= 1000 && code < 2000) return "Anchor IDL error";
  if (code >= 2000 && code < 3000) return "Anchor constraint error";
  if (code >= 3000 && code < 4000) return "Anchor account error";
  if (code >= 4100 && code < 5000) return "Anchor deprecation error";
  return undefined;
}

const PROGRAMS: Idl[] = [STAKING, GOVERNANCE];

/** The program error with `code`, if any Helix program declares it. */
export function programError(
  code: number,
  programId?: string,
): { idl: Idl; name: string; msg?: string } | undefined {
  const candidates = programId ? PROGRAMS.filter((p) => p.address === programId) : PROGRAMS;

  for (const idl of candidates) {
    const found = idl.errors?.find((e) => e.code === code);
    if (found) return { idl, name: found.name, msg: found.msg };
  }
  return undefined;
}

/**
 * Interprets the `err` a simulation or a send returns.
 *
 * The shape is the RPC's, not a library's: `{ InstructionError: [0, { Custom: 6013 }] }`
 * for a program error, a bare string like `"BlockhashNotFound"` for the rest.
 * Everything unrecognised keeps its JSON in `raw`, because a UI that renders
 * "something went wrong" over a message it did not understand destroys the only
 * evidence there was.
 */
export function decodeTransactionError(
  err: unknown,
  logs?: string[] | null,
  programId?: string,
): DecodedError {
  const raw = typeof err === "string" ? err : JSON.stringify(err);

  if (typeof err === "string") {
    return { message: humaniseRuntimeError(err), raw };
  }

  const instructionError = (err as { InstructionError?: [number, unknown] } | null)
    ?.InstructionError;

  if (!Array.isArray(instructionError)) {
    return { message: `The cluster refused the transaction: ${raw}`, raw };
  }

  const [index, detail] = instructionError;

  // A program's own error. The code is what `#[error_code]` assigned.
  const custom = (detail as { Custom?: number } | null)?.Custom;
  if (typeof custom === "number") {
    // Attribute the failure to the program the logs say failed, so a code that
    // exists in two IDLs is not resolved against the wrong one.
    const failing = programId ?? failingProgramFromLogs(logs);

    // Error 0 from the system program is `Allocate: account already in use`,
    // which is what an Anchor `init` returns when the account exists. It is the
    // most reachable failure in this whole UI, and it decodes to nothing at all
    // without this case, because the system program has no IDL.
    //
    // Three different flows land here, and they are the same fact each time: the
    // account this instruction creates is seeded on something already taken. A
    // second vote from one position, a stake against a `position_id` another
    // transaction just claimed, a proposal at an id someone else reached first.
    // Governance's own `UnexpectedProposalId` never fires for that last one —
    // `init` gets there first — which is exactly why this message has to cover
    // it.
    if (custom === 0 && failing === SYSTEM_PROGRAM_ID.toBase58()) {
      return {
        message:
          "That account already exists — a vote from this position, this position id, or " +
          "this proposal id has been taken. Reload and try again.",
        code: 0,
        program: "system",
        instructionIndex: index,
        raw,
      };
    }

    const found = programError(custom, failing);

    if (found) {
      return {
        message: found.msg ?? found.name,
        name: found.name,
        code: custom,
        program: found.idl.metadata.name,
        instructionIndex: index,
        raw,
      };
    }

    const anchor = ANCHOR_ERRORS[custom];
    if (anchor) {
      return { message: anchor, code: custom, instructionIndex: index, raw };
    }

    const range = rangeOf(custom);
    return {
      message: range
        ? `${range} ${custom}. No Helix program declares this code.`
        : `Instruction ${index} failed with program error ${custom} (0x${custom.toString(16)}).`,
      code: custom,
      instructionIndex: index,
      raw,
    };
  }

  // Non-custom instruction errors are runtime conditions, not program logic.
  if (typeof detail === "string") {
    return {
      message: `Instruction ${index} failed: ${humaniseRuntimeError(detail)}`,
      instructionIndex: index,
      raw,
    };
  }

  return { message: `Instruction ${index} failed: ${raw}`, instructionIndex: index, raw };
}

/**
 * A handful of runtime failures phrased for someone holding a wallet.
 *
 * These are conditions of the cluster rather than of the protocol, and the
 * default rendering ("InsufficientFundsForRent") sends people looking at the
 * wrong thing.
 */
function humaniseRuntimeError(name: string): string {
  switch (name) {
    case "BlockhashNotFound":
      return "The blockhash expired before the transaction was signed. Try again.";
    case "AlreadyProcessed":
      return "This exact transaction was already processed.";
    case "InsufficientFundsForRent":
      return "Not enough SOL to pay rent for the account this creates.";
    case "InsufficientFundsForFee":
      return "Not enough SOL to pay the transaction fee.";
    case "ProgramFailedToComplete":
      return "The program ran out of compute budget or aborted.";
    case "AccountNotFound":
      return "An account this transaction reads does not exist on this cluster.";
    case "ProgramAccountNotFound":
      return "The program is not deployed on this cluster. Check the cluster switch.";
    default:
      return name;
  }
}

/**
 * The last program the logs say failed.
 *
 * Simulation logs end with `Program <id> failed: ...` for the program that
 * actually raised, which is more reliable than assuming the top-level program —
 * a treasury transfer fails *inside* the treasury while governance is the
 * program being called.
 */
export function failingProgramFromLogs(logs?: string[] | null): string | undefined {
  if (!logs) return undefined;
  for (let i = logs.length - 1; i >= 0; i--) {
    const match = /^Program (\w{32,44}) failed:/.exec(logs[i]);
    if (match) return match[1];
  }
  return undefined;
}

/**
 * The `Program log:` lines, with the framework's own noise removed.
 *
 * Kept available to the UI rather than hidden: when the decode above cannot name
 * a failure, the logs are what an operator needs, and burying them behind a
 * console is how a bug report becomes "it said error".
 */
export function programLogs(logs?: string[] | null): string[] {
  if (!logs) return [];
  return logs
    .filter((line) => !/^Program \w+ (invoke|success|consumed)/.test(line))
    .map((line) => line.replace(/^Program (log|data): /, ""));
}
