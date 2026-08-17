/**
 * Composing a `ProposalAction`, from the IDL rather than from a hand-written form.
 *
 * `governance/README.md` makes a specific claim about why this enum is closed
 * rather than a blob of serialised CPI data:
 *
 * > the set of things governance *can* do is fixed at deploy time and visible in
 * > the IDL, so a voter reads the variant and knows the blast radius.
 *
 * A form typed out by hand would restate that set in a second place, and the
 * second place is the one that goes stale — a variant added to the program would
 * simply be unreachable from the UI, with nothing failing. So the form is
 * *derived*: [`proposalActions`] reads the variants and their field types out of
 * the IDL, and a new one appears with inputs of the right shape and no edit here.
 *
 * What this deliberately does **not** do is guess semantics. An `i64` is an
 * `i64`: `end_ts` is a moment and `epoch_duration` is a length, and the IDL
 * cannot tell them apart. Rendering a date picker for one and a duration box for
 * the other would be right most of the time, and silently wrong the rest — so
 * both are entered as seconds and [`describeSeconds`] shows both readings at
 * once, leaving the choice with the person who knows which they meant.
 */

import { PublicKey } from "@solana/web3.js";

import type { IdlPrimitive, IdlType } from "./idl.ts";
import { typeDef } from "./idl.ts";
import { GOVERNANCE } from "./programs.ts";

/** How a field should be presented. Derived from its type, never from its name. */
export type InputKind = "address" | "amount" | "seconds" | "number" | "boolean";

export interface ActionField {
  /** The IDL's own path, dotted through one level of nesting: `params.quorum_bps`. */
  path: string;
  /** Last segment, for a label. */
  name: string;
  type: IdlPrimitive;
  input: InputKind;
}

export interface ActionVariant {
  name: string;
  /** Empty for a unit variant like `Signal`. */
  fields: ActionField[];
}

function inputFor(type: IdlPrimitive): InputKind {
  switch (type) {
    case "pubkey":
      return "address";
    case "u64":
    case "u128":
      return "amount";
    case "i64":
    case "i128":
      return "seconds";
    case "bool":
      return "boolean";
    default:
      return "number";
  }
}

/**
 * Flattens a variant's fields, following one level of nested struct.
 *
 * One level, not arbitrary depth, and that is a decision rather than a
 * limitation: `UpdateRealmParams` carries a `RealmParams`, which is the only
 * nesting these programs have. A recursive flattener would handle cases that do
 * not exist and would silently produce an unusable form for a case that has a
 * `Vec` in it. This throws instead, so a shape the form cannot represent is a
 * loud failure rather than a field quietly missing from a governance proposal.
 */
function flatten(prefix: string, type: IdlType, depth = 0): ActionField[] {
  if (typeof type === "string") {
    return [{ path: prefix, name: prefix.split(".").pop()!, type, input: inputFor(type) }];
  }

  if ("defined" in type) {
    if (depth > 0) {
      throw new Error(
        `${prefix} nests ${type.defined.name} more than one level deep — ` +
          `this form cannot represent it, and guessing would drop a field`,
      );
    }
    const def = typeDef(GOVERNANCE, type.defined.name);
    if (def.type.kind !== "struct") {
      throw new Error(`${prefix} is a ${def.name}, which is not a struct`);
    }
    return (def.type.fields ?? []).flatMap((f) =>
      flatten(`${prefix}.${f.name}`, f.type, depth + 1),
    );
  }

  throw new Error(`${prefix} has a type this form cannot represent: ${JSON.stringify(type)}`);
}

/** Every action a proposal may carry, in the order the program declares them. */
export function proposalActions(): ActionVariant[] {
  const def = typeDef(GOVERNANCE, "ProposalAction");
  if (def.type.kind !== "enum") throw new Error("ProposalAction is not an enum");

  return def.type.variants.map((variant) => ({
    name: variant.name,
    fields: (variant.fields ?? []).flatMap((f) => flatten(f.name, f.type)),
  }));
}

// ------------------------------------------------------------------- parsing

export class FieldError extends Error {
  readonly path: string;

  constructor(path: string, message: string) {
    super(`${path}: ${message}`);
    this.path = path;
  }
}

/**
 * Parses one entered value into what the coder wants.
 *
 * Every branch refuses rather than coerces, because the values here end up in a
 * proposal that people vote on and a program later executes. `BigInt("")` is
 * `0n` rather than a throw — the same trap `amount.ts` documents — and a blank
 * `amount` field silently becoming a zero-token treasury transfer is a proposal
 * that says something other than what its author meant.
 */
export function parseField(field: ActionField, raw: string | boolean): unknown {
  if (field.input === "boolean") {
    if (typeof raw !== "boolean") throw new FieldError(field.path, "expected a checkbox value");
    return raw;
  }

  if (typeof raw !== "string") throw new FieldError(field.path, "expected text");
  const value = raw.trim();
  if (value === "") throw new FieldError(field.path, "is required");

  if (field.input === "address") {
    let key: PublicKey;
    try {
      key = new PublicKey(value);
    } catch {
      throw new FieldError(field.path, "is not a valid base58 address");
    }
    // `new PublicKey` accepts 32 bytes of anything, so this is the only check
    // that the string was actually an address rather than base58 of something
    // else that happened to be the right length.
    if (key.toBase58() !== value) {
      throw new FieldError(field.path, "is not a canonical base58 address");
    }
    return key;
  }

  if (!/^-?\d+$/.test(value)) {
    throw new FieldError(field.path, "must be a whole number, with no decimal point");
  }

  // Narrow integers decode as `number` and wide ones as `bigint`; the coder
  // refuses the wrong one, so the split has to match it exactly.
  if (field.type === "u8" || field.type === "u16" || field.type === "u32") {
    if (value.startsWith("-")) throw new FieldError(field.path, "cannot be negative");
    return Number(value);
  }
  if (field.type === "i8" || field.type === "i16" || field.type === "i32") {
    return Number(value);
  }
  return BigInt(value);
}

/**
 * Builds the `{ kind, ... }` object the coder encodes, from a flat form.
 *
 * Rebuilds the nesting the flattening removed, so `params.quorum_bps` lands
 * inside a `params` object where the IDL says it belongs.
 */
export function composeAction(
  variant: ActionVariant,
  values: Record<string, string | boolean>,
): { kind: string; [field: string]: unknown } {
  const action: { kind: string; [field: string]: unknown } = { kind: variant.name };

  for (const field of variant.fields) {
    const parsed = parseField(field, values[field.path] ?? "");
    const [head, nested] = field.path.split(".");

    if (nested === undefined) {
      action[head] = parsed;
    } else {
      const group = (action[head] ??= {}) as Record<string, unknown>;
      group[nested] = parsed;
    }
  }

  return action;
}

// ------------------------------------------------------------------- display

const MINUTE = 60;
const HOUR = 3_600;
const DAY = 86_400;

/**
 * Both readings of an `i64`, because the IDL does not say which was meant.
 *
 * `end_ts` is a moment and `epoch_duration` is a length, and they are the same
 * type. Showing one interpretation would be right most of the time; showing both
 * is right always, and costs a line.
 */
export function describeSeconds(value: string): string | null {
  if (!/^-?\d+$/.test(value)) return null;

  const seconds = Number(value);
  if (!Number.isFinite(seconds) || seconds <= 0) return null;

  const asDuration =
    seconds >= DAY
      ? `${(seconds / DAY).toFixed(seconds % DAY === 0 ? 0 : 1)}d`
      : seconds >= HOUR
        ? `${(seconds / HOUR).toFixed(seconds % HOUR === 0 ? 0 : 1)}h`
        : `${(seconds / MINUTE).toFixed(seconds % MINUTE === 0 ? 0 : 1)}m`;

  // Beyond ~2001 a value is more plausibly a Unix timestamp than a duration, but
  // "more plausibly" is not knowledge, so both are shown either way.
  const asDate = new Date(seconds * 1000).toISOString().replace(".000Z", "Z");

  return `${asDuration} as a duration · ${asDate} as a timestamp`;
}

/** Program bounds worth showing next to a field, from `constants.rs`. */
export const GOVERNANCE_BOUNDS = {
  MAX_TITLE_LEN: 64,
  MAX_URI_LEN: 200,
  MIN_VOTING_PERIOD: 3_600,
  MAX_VOTING_PERIOD: 30 * DAY,
  MIN_TIMELOCK_DELAY: 3_600,
  MAX_TIMELOCK_DELAY: 30 * DAY,
  MAX_BPS: 10_000,
  /** A simple majority is the floor: less would let a minority carry a proposal. */
  MIN_APPROVAL_BPS: 5_001,
} as const;
