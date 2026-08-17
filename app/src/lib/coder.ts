/**
 * Borsh encode/decode driven by the IDL, for both instruction data and account
 * state.
 *
 * ## Why this exists rather than a hand-written layout per instruction
 *
 * The obvious alternative is five small functions that each write a
 * discriminator and a couple of `u64`s. It is less code and it is wrong in a
 * way that does not show up until it matters: an account reordered in a
 * program's `#[derive(Accounts)]` struct, or a new argument, leaves the client
 * compiling and passing its own tests while building a transaction that spends
 * against the wrong account. Anchor's account order is positional and carries no
 * names on the wire, so nothing downstream catches it either — the program sees
 * a well-formed instruction and refuses it for the wrong reason, or worse,
 * accepts it.
 *
 * This is the same failure the indexer hit with its hand-maintained event list
 * (`event_coverage.rs`), which is the reason the answer here is the same: use
 * the artifact `anchor build` generates from the programs themselves.
 *
 * ## Why `bigint` everywhere past 32 bits
 *
 * `u64` and `u128` exceed what a JavaScript number represents exactly, and
 * `amount.ts` exists because a silent rounding in a *display* path is bad. In an
 * *encoding* path it is worse: the rounded value is what gets signed. Every
 * 64-bit and 128-bit field is a `bigint` here and there is no `number` overload
 * for them, so the mistake is a type error rather than a smaller transfer than
 * the user asked for.
 */

import { PublicKey } from "@solana/web3.js";

import type { Idl, IdlField, IdlType, IdlTypeDef } from "./idl.ts";
import { typeDef } from "./idl.ts";

/** A decoded Anchor value. Enums decode as `{ kind, ...fields }`. */
export type Decoded =
  | boolean
  | number
  | bigint
  | string
  | PublicKey
  | Uint8Array
  | null
  | Decoded[]
  | { [field: string]: Decoded };

// ------------------------------------------------------------------- writing

class Writer {
  private chunks: number[] = [];

  bytes(value: Uint8Array): void {
    for (const byte of value) this.chunks.push(byte);
  }

  u8(value: number): void {
    this.chunks.push(value & 0xff);
  }

  /** Little-endian, `width` bytes, from a bigint so 64- and 128-bit are exact. */
  int(value: bigint, width: number, signed: boolean): void {
    const bits = BigInt(width * 8);
    const span = 1n << bits;

    if (signed) {
      const limit = 1n << (bits - 1n);
      if (value < -limit || value >= limit) {
        throw new RangeError(`${value} does not fit in i${width * 8}`);
      }
      if (value < 0n) value += span;
    } else if (value < 0n || value >= span) {
      throw new RangeError(`${value} does not fit in u${width * 8}`);
    }

    for (let i = 0; i < width; i++) {
      this.chunks.push(Number(value & 0xffn));
      value >>= 8n;
    }
  }

  finish(): Uint8Array {
    return Uint8Array.from(this.chunks);
  }
}

// ------------------------------------------------------------------- reading

class Reader {
  private offset = 0;
  private readonly data: Uint8Array;

  // Written out rather than as a parameter property: Node runs this file by
  // stripping types, and `constructor(private data: ...)` is the one TypeScript
  // form that *emits* code rather than deleting it, so it is not strippable.
  constructor(data: Uint8Array) {
    this.data = data;
  }

  get consumed(): number {
    return this.offset;
  }

  get remaining(): number {
    return this.data.length - this.offset;
  }

  bytes(length: number): Uint8Array {
    if (this.offset + length > this.data.length) {
      throw new RangeError(
        `account data ends after ${this.data.length} bytes; ` +
          `wanted ${length} more at offset ${this.offset}`,
      );
    }
    const slice = this.data.subarray(this.offset, this.offset + length);
    this.offset += length;
    return slice;
  }

  u8(): number {
    return this.bytes(1)[0];
  }

  int(width: number, signed: boolean): bigint {
    const raw = this.bytes(width);
    let value = 0n;
    for (let i = width - 1; i >= 0; i--) value = (value << 8n) | BigInt(raw[i]);

    if (signed) {
      const bits = BigInt(width * 8);
      if (value >= 1n << (bits - 1n)) value -= 1n << bits;
    }
    return value;
  }
}

// ---------------------------------------------------------------- primitives

/** Byte width and signedness of each integer primitive. */
const INTEGERS: Record<string, { width: number; signed: boolean }> = {
  u8: { width: 1, signed: false },
  i8: { width: 1, signed: true },
  u16: { width: 2, signed: false },
  i16: { width: 2, signed: true },
  u32: { width: 4, signed: false },
  i32: { width: 4, signed: true },
  u64: { width: 8, signed: false },
  i64: { width: 8, signed: true },
  u128: { width: 16, signed: false },
  i128: { width: 16, signed: true },
};

/**
 * Which integers decode to `number` and which to `bigint`.
 *
 * The split is at 32 bits, where a double stops holding every value. Returning
 * `bigint` for a `u8` would be pedantic; returning `number` for a `u64` would be
 * the bug this module is written to prevent.
 */
const NARROW = new Set(["u8", "i8", "u16", "i16", "u32", "i32"]);

function isDefined(type: IdlType): type is { defined: { name: string } } {
  return typeof type === "object" && "defined" in type;
}

// ------------------------------------------------------------------ encoding

function encodeValue(idl: Idl, type: IdlType, value: unknown, out: Writer): void {
  if (typeof type === "string") {
    const integer = INTEGERS[type];
    if (integer) {
      if (NARROW.has(type)) {
        if (typeof value !== "number" || !Number.isInteger(value)) {
          throw new TypeError(`${type} expects an integer number, got ${describe(value)}`);
        }
        out.int(BigInt(value), integer.width, integer.signed);
      } else {
        // Deliberately not accepting a number here. `BigInt(1e21)` is exact but
        // `BigInt(2 ** 53 + 1)` is not, and an amount is precisely the field
        // where being quietly off by one is a loss.
        if (typeof value !== "bigint") {
          throw new TypeError(`${type} expects a bigint, got ${describe(value)}`);
        }
        out.int(value, integer.width, integer.signed);
      }
      return;
    }

    switch (type) {
      case "bool":
        if (typeof value !== "boolean") throw new TypeError(`bool expects a boolean`);
        out.u8(value ? 1 : 0);
        return;
      case "pubkey": {
        if (!(value instanceof PublicKey)) {
          throw new TypeError(`pubkey expects a PublicKey, got ${describe(value)}`);
        }
        out.bytes(value.toBytes());
        return;
      }
      case "string": {
        if (typeof value !== "string") throw new TypeError(`string expects a string`);
        const utf8 = new TextEncoder().encode(value);
        out.int(BigInt(utf8.length), 4, false);
        out.bytes(utf8);
        return;
      }
      case "bytes": {
        if (!(value instanceof Uint8Array)) throw new TypeError(`bytes expects a Uint8Array`);
        out.int(BigInt(value.length), 4, false);
        out.bytes(value);
        return;
      }
      default:
        throw new Error(`unsupported IDL primitive "${type}"`);
    }
  }

  if ("option" in type) {
    if (value === null || value === undefined) {
      out.u8(0);
      return;
    }
    out.u8(1);
    encodeValue(idl, type.option, value, out);
    return;
  }

  if ("vec" in type) {
    if (!Array.isArray(value)) throw new TypeError(`vec expects an array`);
    out.int(BigInt(value.length), 4, false);
    for (const item of value) encodeValue(idl, type.vec, item, out);
    return;
  }

  if ("array" in type) {
    const [inner, length] = type.array;
    if (!Array.isArray(value) || value.length !== length) {
      throw new TypeError(`array expects exactly ${length} items`);
    }
    for (const item of value) encodeValue(idl, inner, item, out);
    return;
  }

  encodeDefined(idl, typeDef(idl, type.defined.name), value, out);
}

function encodeDefined(idl: Idl, def: IdlTypeDef, value: unknown, out: Writer): void {
  if (def.type.kind === "struct") {
    const fields = def.type.fields ?? [];
    for (const field of fields) {
      encodeValue(idl, field.type, fieldOf(value, field, def.name), out);
    }
    return;
  }

  // Enums are `{ kind: "VariantName", ...fields }`. The tag is the variant's
  // index in the IDL, which is its index in the Rust `enum` — reordering
  // variants is a wire-format change, and the IDL is where that shows up.
  const kind = (value as { kind?: unknown } | null)?.kind;
  if (typeof kind !== "string") {
    throw new TypeError(`${def.name} expects { kind: "..." }, got ${describe(value)}`);
  }

  const index = def.type.variants.findIndex((v) => v.name === kind);
  if (index < 0) {
    throw new Error(
      `${def.name} has no variant "${kind}" — it declares: ` +
        def.type.variants.map((v) => v.name).join(", "),
    );
  }

  out.u8(index);
  for (const field of def.type.variants[index].fields ?? []) {
    encodeValue(idl, field.type, fieldOf(value, field, `${def.name}::${kind}`), out);
  }
}

function fieldOf(value: unknown, field: IdlField, owner: string): unknown {
  if (typeof value !== "object" || value === null) {
    throw new TypeError(`${owner} expects an object, got ${describe(value)}`);
  }
  if (!(field.name in value)) {
    throw new TypeError(`${owner} is missing field "${field.name}"`);
  }
  return (value as Record<string, unknown>)[field.name];
}

function describe(value: unknown): string {
  if (value === null) return "null";
  if (value === undefined) return "undefined";
  if (typeof value === "object") return value.constructor?.name ?? "object";
  return `${typeof value} ${String(value)}`;
}

// ------------------------------------------------------------------ decoding

function decodeValue(idl: Idl, type: IdlType, input: Reader): Decoded {
  if (typeof type === "string") {
    const integer = INTEGERS[type];
    if (integer) {
      const raw = input.int(integer.width, integer.signed);
      return NARROW.has(type) ? Number(raw) : raw;
    }

    switch (type) {
      case "bool": {
        const byte = input.u8();
        // Borsh writes 0 or 1 and nothing else. Anything else means the layout
        // has drifted, and treating it as `true` would hide that.
        if (byte > 1) throw new Error(`bool byte was ${byte}`);
        return byte === 1;
      }
      case "pubkey":
        return new PublicKey(input.bytes(32));
      case "string": {
        const length = Number(input.int(4, false));
        return new TextDecoder().decode(input.bytes(length));
      }
      case "bytes": {
        const length = Number(input.int(4, false));
        return input.bytes(length).slice();
      }
      default:
        throw new Error(`unsupported IDL primitive "${type}"`);
    }
  }

  if ("option" in type) {
    const tag = input.u8();
    if (tag === 0) return null;
    if (tag !== 1) throw new Error(`option tag was ${tag}`);
    return decodeValue(idl, type.option, input);
  }

  if ("vec" in type) {
    const length = Number(input.int(4, false));
    const items: Decoded[] = [];
    for (let i = 0; i < length; i++) items.push(decodeValue(idl, type.vec, input));
    return items;
  }

  if ("array" in type) {
    const [inner, length] = type.array;
    const items: Decoded[] = [];
    for (let i = 0; i < length; i++) items.push(decodeValue(idl, inner, input));
    return items;
  }

  return decodeDefined(idl, typeDef(idl, type.defined.name), input);
}

function decodeDefined(idl: Idl, def: IdlTypeDef, input: Reader): Decoded {
  const out: Record<string, Decoded> = {};

  if (def.type.kind === "struct") {
    for (const field of def.type.fields ?? []) {
      out[field.name] = decodeValue(idl, field.type, input);
    }
    return out;
  }

  const tag = input.u8();
  const variant = def.type.variants[tag];
  if (!variant) throw new Error(`${def.name} has no variant at index ${tag}`);

  out.kind = variant.name;
  for (const field of variant.fields ?? []) {
    out[field.name] = decodeValue(idl, field.type, input);
  }
  return out;
}

// --------------------------------------------------------------------- public

/** Instruction data: the 8-byte discriminator followed by the borsh-encoded args. */
export function encodeInstructionData(
  idl: Idl,
  name: string,
  args: Record<string, unknown>,
): Uint8Array {
  const ix = idl.instructions.find((i) => i.name === name);
  if (!ix) throw new Error(`${idl.metadata.name} has no instruction "${name}"`);

  const out = new Writer();
  out.bytes(Uint8Array.from(ix.discriminator));

  for (const arg of ix.args) {
    if (!(arg.name in args)) {
      throw new TypeError(`${name} is missing argument "${arg.name}"`);
    }
    encodeValue(idl, arg.type, args[arg.name], out);
  }

  // An argument the instruction does not take is nearly always a rename that
  // silently stopped being passed, so the value the caller thinks they are
  // sending is a default. Refuse rather than ignore.
  const declared = new Set(ix.args.map((a) => a.name));
  const extra = Object.keys(args).filter((k) => !declared.has(k));
  if (extra.length > 0) {
    throw new TypeError(`${name} does not take: ${extra.join(", ")}`);
  }

  return out.finish();
}

/**
 * Decodes an account, checking the discriminator first.
 *
 * The check is the point. Every Anchor account is a byte string that will
 * cheerfully decode as some *other* Anchor account of compatible length —
 * `Position` and a truncated `Pool` both start with two pubkeys — and the
 * resulting object looks entirely reasonable. The program does this check too;
 * a client that skips it reports numbers from the wrong account rather than an
 * error.
 */
export function decodeAccount<T = Record<string, Decoded>>(
  idl: Idl,
  name: string,
  data: Uint8Array,
): T {
  const declared = idl.accounts?.find((a) => a.name === name);
  if (!declared) throw new Error(`${idl.metadata.name} declares no account "${name}"`);

  const expected = Uint8Array.from(declared.discriminator);
  if (data.length < expected.length) {
    throw new Error(`account is ${data.length} bytes, too short to be a ${name}`);
  }
  for (let i = 0; i < expected.length; i++) {
    if (data[i] !== expected[i]) {
      throw new Error(
        `account discriminator is not ${name}'s — this is a different account type`,
      );
    }
  }

  const input = new Reader(data.subarray(expected.length));
  return decodeDefined(idl, typeDef(idl, name), input) as T;
}

/** Exposed for tests: encode a standalone value of an IDL type. */
export function encodeType(idl: Idl, type: IdlType, value: unknown): Uint8Array {
  const out = new Writer();
  encodeValue(idl, type, value, out);
  return out.finish();
}

/** Exposed for tests: decode a standalone value, asserting the input is consumed. */
export function decodeType(idl: Idl, type: IdlType, data: Uint8Array): Decoded {
  const input = new Reader(data);
  const value = decodeValue(idl, type, input);
  if (input.remaining !== 0) {
    throw new Error(`${input.remaining} trailing bytes after decoding`);
  }
  return value;
}
