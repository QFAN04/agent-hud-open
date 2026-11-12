import type { HUDRect } from "../geometry";
import type { HUDConfig, HUDEdge } from "../model";

export const EDGE_TAB_THICKNESS = 8;
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

/** Whether a Simple dock is idle and allowed to tuck into a thin edge tab. */
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

/** Slim edge sliver flush against `edge`, centered along `strip`'s span. */
export function retractedStripFrame(
  strip: HUDRect,
  edge: HUDEdge,
  thickness = EDGE_TAB_THICKNESS,
): HUDRect {
  const horiz = edge === "top" || edge === "bottom";
  const longSide = horiz ? strip.width : strip.height;
  const shortSide = horiz ? strip.height : strip.width;
  const thin = Math.max(1, Math.min(thickness, shortSide));
  const span = Math.min(
    longSide,
    Math.max(Math.min(36, longSide), Math.round(longSide * 0.75)),
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
