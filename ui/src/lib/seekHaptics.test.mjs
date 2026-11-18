// Run with: node --test ui/src/lib/seekHaptics.test.mjs
import assert from "node:assert/strict";
import { test } from "node:test";

import { createSeekHaptics } from "./seekHaptics.ts";

test("grain every stepPx", () => {
  const h = createSeekHaptics();
  h.begin(100, 0.5, 0);
  assert.equal(h.move(104, 0.51, 100), null);
  assert.equal(h.move(108, 0.52, 200), "grain");
  assert.equal(h.move(110, 0.53, 300), null);
});

test("rate cap", () => {
  const h = createSeekHaptics();
  h.begin(0, 0.5, 0);
  assert.equal(h.move(50, 0.6, 40), "grain");
  assert.equal(h.move(100, 0.7, 50), null);
  assert.equal(h.move(150, 0.8, 80), "grain");
});

test("one bump per visit to an end", () => {
  const h = createSeekHaptics();
  h.begin(100, 0.5, 0);
  assert.equal(h.move(300, 1, 100), "bump");
  assert.equal(h.move(320, 1, 200), null);
  assert.equal(h.move(310, 1, 300), null);
  assert.equal(h.move(200, 0.8, 400), "grain");
  assert.equal(h.move(300, 1, 500), "bump");
  assert.equal(h.move(0, 0, 600), "bump");
});

test("touch-down on an end does not bump", () => {
  const h = createSeekHaptics();
  h.begin(0, 0, 0);
  assert.equal(h.move(0, 0, 100), null);
});
