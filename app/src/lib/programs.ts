/**
 * Building Helix instructions from the IDL.
 *
 * Nothing here restates a program's account list. `buildInstruction` walks the
 * IDL's accounts in order, derives every PDA from the seed description the IDL
 * carries, and copies `writable`/`signer` from it — so an account added,
 * reordered or made writable in a `#[derive(Accounts)]` struct changes this
 * client the moment `sync-idl` runs, and cannot change it *silently* at all.
 *
 * The accounts the IDL genuinely cannot resolve are the ones the caller has to
 * name: a token account is an arbitrary address, `token_program` is an
 * `Interface` that could be either token program, and a PDA seeded from another
 * account's *contents* (`realm.staking_pool`) needs those contents. Everything
 * else is derived, and asking for an account that is derivable is an error
 * rather than an override — an override is how a client ends up signing against
 * a look-alike PDA.
 */

import { PublicKey, TransactionInstruction, type AccountMeta } from "@solana/web3.js";
// Explicitly, not from a global. `Buffer` is global in Node and is not in the
// browser; web3.js imports it the same way, so this resolves to the same
// polyfill the bundler already includes rather than a second copy.
import { Buffer } from "buffer";

import governanceIdl from "../idl/helix_governance.ts";
import stakingIdl from "../idl/helix_staking.ts";
import { encodeInstructionData, encodeType } from "./coder.ts";
import type { Idl, IdlPrimitive, IdlSeed, IdlType } from "./idl.ts";
import { instruction, typeDef } from "./idl.ts";

export const STAKING: Idl = stakingIdl;
export const GOVERNANCE: Idl = governanceIdl;

/**
 * Program ids come from the IDL, which comes from `declare_id!`.
 *
 * Not from an environment variable and not from a constant typed in here: a
 * dashboard pointed at a program id that is not the one the IDL describes builds
 * correctly-shaped instructions for the wrong program, and the only symptom is
 * a failure that looks like a bug in the program.
 */
export const STAKING_PROGRAM_ID = new PublicKey(STAKING.address);
export const GOVERNANCE_PROGRAM_ID = new PublicKey(GOVERNANCE.address);

export const SYSTEM_PROGRAM_ID = new PublicKey("11111111111111111111111111111111");
export const TOKEN_PROGRAM_ID = new PublicKey("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
export const TOKEN_2022_PROGRAM_ID = new PublicKey("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");
export const ASSOCIATED_TOKEN_PROGRAM_ID = new PublicKey(
  "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL",
);

// --------------------------------------------------------------- token helpers

/**
 * The associated token account for `(owner, mint)` under a given token program.
 *
 * The token program is a parameter rather than a default because these programs
 * take `Interface<TokenInterface>`: the same mint address under Token and
 * Token-2022 gives different ATAs, and defaulting to the classic one would
 * quietly produce an account the transfer cannot use. Read the mint account's
 * owner and pass it; `chain.ts` does.
 */
export function associatedTokenAddress(
  owner: PublicKey,
  mint: PublicKey,
  tokenProgram: PublicKey,
): PublicKey {
  const [address] = PublicKey.findProgramAddressSync(
    [owner.toBytes(), tokenProgram.toBytes(), mint.toBytes()],
    ASSOCIATED_TOKEN_PROGRAM_ID,
  );
  return address;
}

/**
 * `CreateIdempotent` on the associated-token-account program.
 *
 * Idempotent rather than `Create`, and always prepended rather than prepended
 * only when the account is missing. Checking first is a read that can be stale
 * by the time the transaction lands — two claims signed in the same breath, and
 * the second one fails on an account the first just created. The instruction
 * costs a few hundred compute units when the account already exists.
 */
export function createAssociatedTokenAccountIdempotentIx(
  payer: PublicKey,
  owner: PublicKey,
  mint: PublicKey,
  tokenProgram: PublicKey,
): TransactionInstruction {
  return new TransactionInstruction({
    programId: ASSOCIATED_TOKEN_PROGRAM_ID,
    keys: [
      { pubkey: payer, isSigner: true, isWritable: true },
      {
        pubkey: associatedTokenAddress(owner, mint, tokenProgram),
        isSigner: false,
        isWritable: true,
      },
      { pubkey: owner, isSigner: false, isWritable: false },
      { pubkey: mint, isSigner: false, isWritable: false },
      { pubkey: SYSTEM_PROGRAM_ID, isSigner: false, isWritable: false },
      { pubkey: tokenProgram, isSigner: false, isWritable: false },
    ],
    // Instruction 1 of the ATA program. A single tag byte, no arguments.
    data: Buffer.from([1]),
  });
}

// ------------------------------------------------------------ pda derivation

/**
 * Types that may be used as a PDA seed.
 *
 * Fixed-width only, and notably *not* `string`: borsh prefixes a string with its
 * length and `Pubkey::find_program_address` does not, so encoding one through
 * the coder would derive an address the program never will. None of these
 * programs seeds on a string — `position_id` is a `u64` precisely so seeds stay
 * fixed-length (see `Position::position_id`) — and if one ever does, this has to
 * be taught the difference rather than left to guess.
 */
const SEEDABLE_TYPES = new Set([
  "u8",
  "i8",
  "u16",
  "i16",
  "u32",
  "i32",
  "u64",
  "i64",
  "u128",
  "i128",
  "pubkey",
]);

/** Anything that can stand in for a seed value the IDL cannot derive. */
export type SeedValue = PublicKey | bigint | number;

function seedable(type: IdlType, context: string): IdlPrimitive {
  if (typeof type !== "string" || !SEEDABLE_TYPES.has(type)) {
    throw new Error(`${context}: cannot use a ${JSON.stringify(type)} as a PDA seed`);
  }
  return type as IdlPrimitive;
}

/**
 * Widens a supplied seed value to what the coder wants for `type`.
 *
 * A `number` is accepted for a 64-bit field only while it is exactly
 * representable. Past 2^53 it is not, and silently deriving a PDA from a rounded
 * id would send the transaction at a different account than the caller named —
 * so that case throws and says to pass a `bigint`.
 */
function coerce(value: SeedValue, type: IdlPrimitive): unknown {
  const wide = type === "u64" || type === "i64" || type === "u128" || type === "i128";
  if (wide && typeof value === "number") {
    if (!Number.isSafeInteger(value)) {
      throw new RangeError(`${value} is not exact as a number — pass a bigint for a ${type} seed`);
    }
    return BigInt(value);
  }
  return value;
}

/** The declared type of `Account.field`, so a supplied seed encodes as the chain does. */
function fieldType(idl: Idl, accountType: string, field: string, context: string): IdlType {
  const def = typeDef(idl, accountType);
  if (def.type.kind !== "struct") {
    throw new Error(`${context}: ${accountType} is not a struct`);
  }
  const found = def.type.fields?.find((f) => f.name === field);
  if (!found) throw new Error(`${context}: ${accountType} has no field "${field}"`);
  return found.type;
}

export interface BuildOptions {
  /**
   * Accounts the IDL cannot derive: token accounts, `token_program`, and
   * anything the caller already knows. Naming an account the IDL *can* derive
   * is refused — see the module docs.
   */
  accounts?: Record<string, PublicKey>;
  /** Instruction arguments, keyed by the IDL's argument names. */
  args?: Record<string, unknown>;
  /**
   * Values for seeds that read a field out of another account, keyed by the
   * IDL's own path — e.g. `"pool.stake_mint"` or `"position.position_id"`.
   * These come from a decoded account, so they are facts read off the chain
   * rather than guesses, and each is encoded with the type the IDL gives that
   * field: a pubkey as 32 bytes, a `u64` as eight little-endian ones.
   */
  seedFields?: Record<string, SeedValue>;
}

function seedBytes(
  idl: Idl,
  seed: IdlSeed,
  resolved: Map<string, PublicKey>,
  options: BuildOptions,
  argTypes: Map<string, IdlType>,
  context: string,
): Uint8Array {
  switch (seed.kind) {
    case "const":
      if (!seed.value) throw new Error(`${context}: const seed without a value`);
      return Uint8Array.from(seed.value);

    case "account": {
      const path = seed.path;
      if (!path) throw new Error(`${context}: account seed without a path`);

      // A bare name refers to another account of this instruction.
      if (!path.includes(".")) {
        const address = resolved.get(path);
        if (!address) {
          throw new Error(
            `${context}: seeded on account "${path}", which is not resolved yet. ` +
              `The IDL lists it after this account, so it has to be supplied.`,
          );
        }
        return address.toBytes();
      }

      // A dotted path reads a field out of an account's *data*, which is not
      // available from the instruction alone and has to be read off the chain.
      const supplied = options.seedFields?.[path];
      if (supplied === undefined) {
        throw new Error(
          `${context}: seeded on "${path}", a field of another account. ` +
            `Pass it as seedFields["${path}"] — read it off the chain, do not assume it.`,
        );
      }
      if (!seed.account) throw new Error(`${context}: seed "${path}" names no account type`);

      // Encoded with the type the IDL gives that field, so a `u64` id becomes
      // eight little-endian bytes and a pubkey becomes thirty-two. Guessing
      // from the JavaScript value would get `position_id` wrong the moment it
      // arrived as a number.
      const declared = seedable(fieldType(idl, seed.account, path.split(".")[1], context), context);
      return encodeType(idl, declared, coerce(supplied, declared));
    }

    case "arg": {
      const name = seed.path;
      if (!name) throw new Error(`${context}: arg seed without a path`);

      const type = argTypes.get(name);
      if (!type) throw new Error(`${context}: seeded on unknown argument "${name}"`);
      const declared = seedable(type, context);

      const value = options.args?.[name];
      if (value === undefined) throw new Error(`${context}: argument "${name}" is required`);
      return encodeType(idl, declared, value);
    }

    default:
      throw new Error(`${context}: unsupported seed kind "${String(seed.kind)}"`);
  }
}

/**
 * Resolves an instruction's accounts in IDL order.
 *
 * `stopAt` returns as soon as that account is resolved, so deriving an early PDA
 * does not require supplying the arbitrary accounts that come after it — the
 * pool's address can be derived without naming a token account.
 */
function resolveAccounts(
  idl: Idl,
  programId: PublicKey,
  name: string,
  options: BuildOptions,
  stopAt?: string,
): { resolved: Map<string, PublicKey>; keys: AccountMeta[] } {
  const ix = instruction(idl, name);
  const argTypes = new Map(ix.args.map((a) => [a.name, a.type]));

  const resolved = new Map<string, PublicKey>();
  const keys: AccountMeta[] = [];

  for (const account of ix.accounts) {
    const context = `${idl.metadata.name}.${name}:${account.name}`;
    const supplied = options.accounts?.[account.name];

    let address: PublicKey;
    if (account.address) {
      // A fixed address in the IDL — the system program and friends. Supplying
      // it is harmless but pointless; supplying something *else* is a bug.
      address = new PublicKey(account.address);
      if (supplied && !supplied.equals(address)) {
        throw new Error(`${context}: the IDL fixes this account at ${account.address}`);
      }
    } else if (account.pda) {
      if (supplied) {
        throw new Error(
          `${context}: this is a PDA the IDL describes, so it is derived, not passed. ` +
            `Passing it would let a look-alike address through.`,
        );
      }
      const seeds = account.pda.seeds.map((seed) =>
        seedBytes(idl, seed, resolved, options, argTypes, context),
      );
      [address] = PublicKey.findProgramAddressSync(
        seeds.map((s) => Buffer.from(s)),
        programId,
      );
    } else {
      if (!supplied) throw new Error(`${context}: must be supplied`);
      address = supplied;
    }

    resolved.set(account.name, address);
    keys.push({
      pubkey: address,
      isSigner: account.signer === true,
      isWritable: account.writable === true,
    });

    if (stopAt === account.name) return { resolved, keys };
  }

  if (stopAt) throw new Error(`${idl.metadata.name}.${name} has no account "${stopAt}"`);

  const unknown = Object.keys(options.accounts ?? {}).filter((k) => !resolved.has(k));
  if (unknown.length > 0) {
    throw new Error(`${idl.metadata.name}.${name} has no account named: ${unknown.join(", ")}`);
  }

  return { resolved, keys };
}

/** Builds an instruction, deriving everything the IDL says how to derive. */
export function buildInstruction(
  idl: Idl,
  programId: PublicKey,
  name: string,
  options: BuildOptions = {},
): TransactionInstruction {
  const { keys } = resolveAccounts(idl, programId, name, options);

  return new TransactionInstruction({
    programId,
    keys,
    data: Buffer.from(encodeInstructionData(idl, name, options.args ?? {})),
  });
}

/** All addresses an instruction resolves to, without building it. */
export function resolvedAccounts(
  idl: Idl,
  programId: PublicKey,
  name: string,
  options: BuildOptions = {},
): Record<string, PublicKey> {
  return Object.fromEntries(resolveAccounts(idl, programId, name, options).resolved);
}

/**
 * One account's address, derived from the IDL's seeds.
 *
 * Exists so nothing outside this module ever writes a seed literal. A `"pool"`
 * typed in by hand somewhere else is the drift this whole file is arranged to
 * prevent, and it would go unnoticed because it would be *right* until the
 * program changed.
 */
export function derivePda(
  idl: Idl,
  programId: PublicKey,
  instructionName: string,
  accountName: string,
  options: BuildOptions = {},
): PublicKey {
  const { resolved } = resolveAccounts(idl, programId, instructionName, options, accountName);
  const address = resolved.get(accountName);
  if (!address) throw new Error(`${instructionName} did not resolve "${accountName}"`);
  return address;
}
