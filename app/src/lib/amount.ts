/**
 * Formatting base-unit amounts that arrive as strings.
 *
 * The API sends amounts as strings on purpose: they are `u64` on chain, and JSON
 * numbers are IEEE-754 doubles that stop being exact above 2^53. For a 9-decimal
 * mint that limit is about nine million tokens.
 *
 * That care is wasted if the client's first move is `Number(response.total_staked)`,
 * which is exactly what happens by default. Everything here goes through `BigInt`,
 * and `toDisplay` does its own decimal placement rather than dividing — dividing
 * by `10 ** decimals` puts the value back into a double and undoes the point of
 * the string.
 */

/** Thousands separators, applied to a digit string rather than a number. */
function groupDigits(digits: string): string {
  return digits.replace(/\B(?=(\d{3})+(?!\d))/g, ",");
}

/**
 * Renders base units as a decimal string.
 *
 * @param raw base units, as the API sends them
 * @param decimals the mint's decimals (9 for HLX)
 * @param maxFractionDigits trailing digits to keep; the rest are truncated, not
 *   rounded, so a displayed balance is never larger than the real one
 */
export function toDisplay(raw: string, decimals = 9, maxFractionDigits = 4): string {
  // `BigInt("")` is `0n` rather than a throw, and `BigInt(" 12 ")` is 12 — so a
  // try/catch alone would render a missing field as a confident zero, which is
  // the exact failure this module exists to avoid. Match the shape first.
  if (!/^-?\d+$/.test(raw)) return "—";

  let value: bigint;
  try {
    value = BigInt(raw);
  } catch {
    return "—";
  }

  const negative = value < 0n;
  if (negative) value = -value;

  const scale = 10n ** BigInt(decimals);
  const whole = value / scale;
  const fraction = (value % scale).toString().padStart(decimals, "0");

  const kept = fraction.slice(0, maxFractionDigits).replace(/0+$/, "");
  const sign = negative ? "-" : "";

  return kept ? `${sign}${groupDigits(whole.toString())}.${kept}` : `${sign}${groupDigits(whole.toString())}`;
}

/**
 * Basis points as a percentage string.
 *
 * `null` is not zero. The API returns `null` for an APR over an empty pool
 * because the figure is undefined, and rendering that as `0.00%` would put a
 * confident wrong number on the screen.
 */
export function bpsToPercent(bps: number | null | undefined, fractionDigits = 2): string {
  if (bps === null || bps === undefined) return "—";
  return `${(bps / 100).toFixed(fractionDigits)}%`;
}

/** Truncates an address for display, keeping both ends so it stays identifiable. */
export function shortAddress(address: string, lead = 4, tail = 4): string {
  if (address.length <= lead + tail + 1) return address;
  return `${address.slice(0, lead)}…${address.slice(-tail)}`;
}
