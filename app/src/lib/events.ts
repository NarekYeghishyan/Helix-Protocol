/**
 * Decoding the events a (simulated) transaction emitted.
 *
 * This is how the UI answers "how much will I actually receive?" without
 * implementing the reward maths a second time.
 *
 * The tempting alternative is to reimplement `Position::earned` in TypeScript:
 * read `pool.reward_per_token`, advance it to now, multiply by the position's
 * weight. It is twenty lines and it is a second implementation of the thing the
 * program is authoritative about — the exact mistake the indexer was corrected
 * for in Phase 4.0, where `Unstaked` had to start carrying the position's
 * remaining weight so the projection would stop re-deriving it from the tier
 * table (`ROADMAP` 4.0, W-8). A second implementation agrees right up until the
 * moment one of them changes.
 *
 * A simulated `claim` already computed the number, and `RewardsClaimed` carries
 * it. So the preview reports the amount the program transferred in simulation,
 * not an estimate of it. If the two would ever disagree, there is nothing here
 * to disagree with.
 */

import { Buffer } from "buffer";

import { decodeType } from "./coder.ts";
import type { Decoded } from "./coder.ts";
import type { Idl } from "./idl.ts";

export interface DecodedEvent {
  /** The event type's name, e.g. `RewardsClaimed`. */
  name: string;
  program: string;
  fields: Record<string, Decoded>;
}

/**
 * Anchor's `emit!` writes the event as a base64 `Program data:` log line:
 * an 8-byte discriminator followed by the borsh-encoded struct.
 */
const DATA_LINE = /^Program data: (.+)$/;

function sameBytes(a: Uint8Array, b: Uint8Array, length: number): boolean {
  for (let i = 0; i < length; i++) if (a[i] !== b[i]) return false;
  return true;
}

/**
 * Every Helix event in `logs`, in the order the runtime logged them.
 *
 * Lines that are not events, and events belonging to programs not passed in,
 * are skipped rather than reported: the logs of any real transaction contain
 * `Program data:` lines from token programs too, and treating an unrecognised
 * one as an error would make every successful claim look broken.
 */
export function decodeEvents(idls: Idl[], logs?: string[] | null): DecodedEvent[] {
  if (!logs) return [];

  const found: DecodedEvent[] = [];

  for (const line of logs) {
    const match = DATA_LINE.exec(line);
    if (!match) continue;

    let payload: Uint8Array;
    try {
      payload = Buffer.from(match[1], "base64");
    } catch {
      continue;
    }
    if (payload.length < 8) continue;

    for (const idl of idls) {
      const declared = idl.events?.find((e) =>
        sameBytes(payload, Uint8Array.from(e.discriminator), 8),
      );
      if (!declared) continue;

      try {
        const fields = decodeType(idl, { defined: { name: declared.name } }, payload.subarray(8));
        found.push({
          name: declared.name,
          program: idl.metadata.name,
          fields: fields as Record<string, Decoded>,
        });
      } catch {
        // A discriminator that matches but a body that does not decode means
        // this build's IDL is not the deployed program's. Skipping would hide
        // that, so it is surfaced as an event with no fields rather than
        // dropped — the name alone is enough to notice the mismatch.
        found.push({ name: declared.name, program: idl.metadata.name, fields: {} });
      }
      break;
    }
  }

  return found;
}

/** The first event of a given type, if the transaction emitted one. */
export function firstEvent(events: DecodedEvent[], name: string): DecodedEvent | undefined {
  return events.find((e) => e.name === name);
}

/** A `u64`/`u128` field, as the base-unit string the rest of the UI formats. */
export function amountField(event: DecodedEvent | undefined, field: string): string | undefined {
  const value = event?.fields[field];
  return typeof value === "bigint" ? value.toString() : undefined;
}
