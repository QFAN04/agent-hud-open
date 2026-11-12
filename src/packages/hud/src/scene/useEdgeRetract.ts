import { useEffect, useState } from "react";
import type { HUDRect } from "../geometry";
import type { HUDEdge } from "../model";
import {
  canRetractAtEdge,
  EDGE_RETRACT_DELAY_MS,
  retractedStripFrame,
  type EdgeRetractGate,
} from "./edge-retract";

/** Manages auto-retracting a docked Simple strip into an 8px edge sliver when
 * idle, and waking immediately on hover, alert, open panel, or drag. */
export function useEdgeRetract(
  gate: EdgeRetractGate,
  strip: HUDRect,
  edge: HUDEdge,
) {
  const allowed = canRetractAtEdge(gate);
  const [retracted, setRetracted] = useState(false);

  useEffect(() => {
    if (!allowed) {
      setRetracted(false);
      return;
    }
    const timer = setTimeout(() => setRetracted(true), EDGE_RETRACT_DELAY_MS);
    return () => clearTimeout(timer);
  }, [allowed]);

  return {
    retracted,
    effectiveStrip: retracted ? retractedStripFrame(strip, edge) : strip,
  };
}
