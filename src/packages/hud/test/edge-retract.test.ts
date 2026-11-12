import { test } from "node:test";
import assert from "node:assert/strict";
import {
  COMPACT_EDGE_PADDING,
  COMPACT_MARK_GAP,
  COMPACT_MARK_SIZE,
  COMPACT_STRIP_THICKNESS,
  EDGE_RETRACT_DELAY_MS,
  canRetractAtEdge,
  retractedStripFrame,
  type EdgeRetractGate,
} from "../src/scene/edge-retract.ts";

const idleGate: EdgeRetractGate = {
  mode: "simple",
  autoHideAtEdge: true,
  preview: false,
  open: false,
  pinned: false,
  pointerInside: false,
  cardVisible: false,
  dragVisual: false,
  hasAlert: false,
  hasOpenEnd: false,
};

test("canRetractAtEdge only enables retract for idle non-preview Simple docks", () => {
  assert.equal(COMPACT_MARK_SIZE, 18);
  assert.equal(COMPACT_MARK_GAP, 5);
  assert.equal(COMPACT_EDGE_PADDING, 5);
  assert.equal(COMPACT_STRIP_THICKNESS, 28);
  assert.equal(EDGE_RETRACT_DELAY_MS, 350);
  assert.equal(canRetractAtEdge(idleGate), true);

  for (const [label, override] of [
    ["notch mode", { mode: "notch" as const }],
    ["setting disabled", { autoHideAtEdge: false }],
    ["drag/wallpaper preview", { preview: true }],
    ["panel open", { open: true }],
    ["panel pinned", { pinned: true }],
    ["pointer inside", { pointerInside: true }],
    ["card visible", { cardVisible: true }],
    ["dragging", { dragVisual: true }],
    ["active alert", { hasAlert: true }],
    ["open dock end", { hasOpenEnd: true }],
  ] as const) {
    assert.equal(
      canRetractAtEdge({ ...idleGate, ...override }),
      false,
      `should not retract when ${label}`,
    );
  }
});

test("retractedStripFrame sizes a 28px Dynamic-style compact strip for live bot sprites", () => {
  // 4 marks: 5*2 + 4*18 + 3*5 = 97px span centered inside the full strip.
  assert.deepEqual(
    retractedStripFrame(
      { x: 1840, y: 300, width: 80, height: 140 },
      "right",
      4,
    ),
    { x: 1892, y: 322, width: 28, height: 97 },
  );
  assert.deepEqual(
    retractedStripFrame({ x: 0, y: 300, width: 80, height: 140 }, "left", 4),
    { x: 0, y: 322, width: 28, height: 97 },
  );
  // 1 mark: 5*2 + 18 = 28px square-pill flush at edge.
  assert.deepEqual(
    retractedStripFrame({ x: 400, y: 0, width: 200, height: 48 }, "top", 1),
    { x: 486, y: 0, width: 28, height: 28 },
  );
  assert.deepEqual(
    retractedStripFrame(
      { x: 400, y: 1032, width: 200, height: 48 },
      "bottom",
      1,
    ),
    { x: 486, y: 1052, width: 28, height: 28 },
  );
  // 0 vendors falls back to 36px span and short strips clamp cleanly.
  assert.deepEqual(
    retractedStripFrame(
      { x: 1840, y: 300, width: 80, height: 140 },
      "right",
      0,
    ),
    { x: 1892, y: 352, width: 28, height: 36 },
  );
  assert.deepEqual(
    retractedStripFrame({ x: 1880, y: 500, width: 40, height: 32 }, "right", 0),
    { x: 1892, y: 500, width: 28, height: 32 },
  );
  assert.deepEqual(
    retractedStripFrame({ x: 1890, y: 500, width: 20, height: 24 }, "right", 4),
    { x: 1890, y: 500, width: 20, height: 24 },
  );
});
