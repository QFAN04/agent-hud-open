import type { HUDRect } from "../geometry";
import type { HUDConfig, HUDEdge } from "../model";

export const COMPACT_MARK_SIZE = 18;
export const COMPACT_MARK_GAP = 5;
export const COMPACT_EDGE_PADDING = 5;
export const COMPACT_STRIP_THICKNESS =
  COMPACT_MARK_SIZE + COMPACT_EDGE_PADDING * 2;
export const EDGE_RETRACT_DELAY_MS = 350;

export interface EdgeRetractGate {
  mode: HUDConfig["mode"];
  autoHideAtEdge: boolean;
  preview: boolean;
  open: boolean;
  pinned: boolean;
  pointerInside: boolean;
  cardVisible: boolean;
  dragVisual: boolean;
  hasAlert: boolean;
  hasOpenEnd: boolean;
}

/** Whether a Simple dock is idle and allowed to tuck into a compact edge strip. */
export function canRetractAtEdge(gate: EdgeRetractGate): boolean {
  return (
    gate.mode === "simple" &&
    gate.autoHideAtEdge &&
    !gate.preview &&
    !gate.open &&
    !gate.pinned &&
    !gate.pointerInside &&
    !gate.cardVisible &&
    !gate.dragVisual &&
    !gate.hasAlert &&
    !gate.hasOpenEnd
  );
}

/** Compact Dynamic-style strip flush against `edge`, centered along `strip`. */
export function retractedStripFrame(
  strip: HUDRect,
  edge: HUDEdge,
  vendorCount = 0,
  thickness = COMPACT_STRIP_THICKNESS,
): HUDRect {
  const horiz = edge === "top" || edge === "bottom";
  const longSide = horiz ? strip.width : strip.height;
  const shortSide = horiz ? strip.height : strip.width;
  const thin = Math.max(1, Math.min(thickness, shortSide));
  const desiredSpan =
    vendorCount > 0
      ? COMPACT_EDGE_PADDING * 2 +
        vendorCount * COMPACT_MARK_SIZE +
        (vendorCount - 1) * COMPACT_MARK_GAP
      : 36;
  const span = Math.min(
    longSide,
    Math.max(Math.min(thin, longSide), desiredSpan),
  );
  switch (edge) {
    case "left":
      return {
        x: strip.x,
        y: Math.round(strip.y + (strip.height - span) / 2),
        width: thin,
        height: span,
      };
    case "right":
      return {
        x: strip.x + strip.width - thin,
        y: Math.round(strip.y + (strip.height - span) / 2),
        width: thin,
        height: span,
      };
    case "top":
      return {
        x: Math.round(strip.x + (strip.width - span) / 2),
        y: strip.y,
        width: span,
        height: thin,
      };
    case "bottom":
      return {
        x: Math.round(strip.x + (strip.width - span) / 2),
        y: strip.y + strip.height - thin,
        width: span,
        height: thin,
      };
  }
}
