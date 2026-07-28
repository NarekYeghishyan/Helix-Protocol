/**
 * Run with `npm test` — Node's built-in runner, no test framework installed.
 * Node 24 strips TypeScript types natively, so these need no build step either.
 */
import { strict as assert } from "node:assert";
import { test } from "node:test";

import { bpsToPercent, shortAddress, toDisplay } from "./amount.ts";

test("an amount past the double-precision limit is exact", () => {
  // 2^53 + 1, the smallest integer a JSON number cannot represent.
  const raw = "9007199254740993";

  assert.equal(toDisplay(raw, 0, 0), "9,007,199,254,740,993");

  // And the mistake this guards against: the same value through a Number.
  assert.notEqual(String(Number(raw)), raw);
});

test("decimals are placed without dividing", () => {
  assert.equal(toDisplay("1000000000", 9), "1");
  assert.equal(toDisplay("1500000000", 9), "1.5");
  assert.equal(toDisplay("1234567890", 9), "1.2345");
  assert.equal(toDisplay("0", 9), "0");
});

test("fractions truncate rather than round, so a balance is never overstated", () => {
  // 1.99999 would round to 2.0 at four places; a wallet showing more than the
  // user has is worse than one showing slightly less.
  assert.equal(toDisplay("1999990000", 9, 4), "1.9999");
});

test("a sub-display-precision balance is not shown as zero-point-nothing", () => {
  // 1 base unit of a 9-decimal mint. Truncation leaves no fraction digits, so
  // the whole part stands alone rather than rendering "0.".
  assert.equal(toDisplay("1", 9, 4), "0");
});

test("large balances are grouped", () => {
  assert.equal(toDisplay("123456789000000000", 9, 0), "123,456,789");
});

test("malformed input is a dash, not a crash or a zero", () => {
  assert.equal(toDisplay("not-a-number"), "—");
  // `BigInt("")` is 0n and `BigInt(" 12 ")` is 12n, so a try/catch alone would
  // have rendered a missing field as a confident "0". This test found that.
  assert.equal(toDisplay(""), "—");
  assert.equal(toDisplay("  "), "—");
  assert.equal(toDisplay("12.5"), "—", "a decimal is not base units");
  assert.equal(toDisplay("0x10"), "—");
});

test("an undefined APR is a dash, never 0.00%", () => {
  assert.equal(bpsToPercent(null), "—");
  assert.equal(bpsToPercent(undefined), "—");
  assert.equal(bpsToPercent(0), "0.00%");
  assert.equal(bpsToPercent(1234), "12.34%");
});

test("addresses keep both ends so they stay identifiable", () => {
  const address = "So11111111111111111111111111111111111111112";
  const short = shortAddress(address);
  assert.ok(short.startsWith("So11"));
  assert.ok(short.endsWith("1112"));
  assert.equal(shortAddress("short"), "short");
});
