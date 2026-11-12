import { test } from "node:test";
import assert from "node:assert/strict";
import {
  EDGE_RETRACT_DELAY_MS,
  EDGE_TAB_THICKNESS,
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
  assert.equal(EDGE_TAB_THICKNESS, 8);
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

test("retractedStripFrame creates an 8px flush edge sliver centered along the strip", () => {
  assert.deepEqual(
    retractedStripFrame({ x: 1840, y: 300, width: 80, height: 120 }, "right"),
    { x: 1912, y: 315, width: 8, height: 90 },
  );
  assert.deepEqual(
    retractedStripFrame({ x: 0, y: 300, width: 80, height: 120 }, "left"),
    { x: 0, y: 315, width: 8, height: 90 },
  );
  assert.deepEqual(
    retractedStripFrame({ x: 400, y: 0, width: 200, height: 32 }, "top"),
    { x: 425, y: 0, width: 150, height: 8 },
  );
  assert.deepEqual(
    retractedStripFrame({ x: 400, y: 1048, width: 200, height: 32 }, "bottom"),
    { x: 425, y: 1072, width: 150, height: 8 },
  );
  // Short strips keep their full span without shrinking below 36px or overflowing.
  assert.deepEqual(
    retractedStripFrame({ x: 1880, y: 500, width: 40, height: 32 }, "right"),
    { x: 1912, y: 500, width: 8, height: 32 },
  );
});
