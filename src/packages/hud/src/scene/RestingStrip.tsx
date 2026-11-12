import {
  useEffect,
  useRef,
  type MutableRefObject,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
} from "react";
import { motion } from "motion/react";
import { position, type HUDRect } from "../geometry";
import type { HUDEdge, HUDVendor } from "../model";
import type { dragSurfaceGeometry } from "../placement";
import { EdgeTab } from "./EdgeTab";
import type { HUDNativePointer } from "./useHUDInteraction";

function edgeSlideInitial(edge: HUDEdge) {
  switch (edge) {
    case "left":
      return { x: -12, y: 0, opacity: 0.55 };
    case "right":
      return { x: 12, y: 0, opacity: 0.55 };
    case "top":
      return { x: 0, y: -12, opacity: 0.55 };
    case "bottom":
      return { x: 0, y: 12, opacity: 0.55 };
  }
}

/** The resting strip the marks sit in: it takes the pointer for them, with
 * the surface a drag grabs it by and, while it is dragged, its outline. */
export function RestingStrip({
  strip,
  grab,
  draggable,
  dragVisual,
  retracted = false,
  edge,
  vendors,
  fill,
  outline,
  light,
  reduceMotion,
  considerDrag,
  suppressDragClick,
  nativePointer,
  followMark,
  entered,
  domExited,
  openAtTop,
  vendorUnderPointer,
  hovered,
  pin,
  children,
}: {
  strip: HUDRect;
  grab: ReturnType<typeof dragSurfaceGeometry>;
  /** The host moves the HUD: the strip starts a drag. */
  draggable: boolean;
  dragVisual: boolean;
  retracted?: boolean;
  edge: HUDEdge;
  vendors: readonly HUDVendor[];
  fill: string;
  outline: string;
  light: boolean;
  reduceMotion: boolean;
  considerDrag: (event: ReactPointerEvent<HTMLDivElement>) => void;
  suppressDragClick: MutableRefObject<boolean>;
  nativePointer?: HUDNativePointer;
  followMark: (event: { clientX: number; clientY: number }) => void;
  entered: (optionDown?: boolean) => void;
  domExited: (event?: { relatedTarget?: EventTarget | null }) => void;
  openAtTop: (event: {
    clientX: number;
    clientY: number;
    altKey: boolean | null;
  }) => void;
  vendorUnderPointer: (event: {
    clientX: number;
    clientY: number;
  }) => string | null;
  hovered: string | null;
  pin: (key?: string) => void;
  /** The marks. */
  children: ReactNode;
}) {
  const wasRetracted = useRef(false);
  const slideFromRetract = !retracted && wasRetracted.current && !reduceMotion;
  useEffect(() => {
    wasRetracted.current = retracted;
  }, [retracted]);
  const grabBox = retracted
    ? { left: 0, top: 0, width: strip.width, height: strip.height }
    : {
        left: grab.frame.x - strip.x,
        top: grab.frame.y - strip.y,
        width: grab.frame.width,
        height: grab.frame.height,
      };
  return (
    <>
      <div
        className="hud-resting-strip"
        style={{
          ...position(strip),
          zIndex: 3,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          overflow: retracted ? "hidden" : undefined,
          userSelect: "none",
          touchAction: draggable ? "none" : undefined,
          cursor: dragVisual ? "grabbing" : undefined,
        }}
        onPointerDownCapture={considerDrag}
        onClickCapture={(event) => {
          if (!suppressDragClick.current || event.detail === 0) return;
          event.preventDefault();
          event.stopPropagation();
          suppressDragClick.current = false;
        }}
        onPointerEnter={(e) => {
          if (!nativePointer) {
            followMark(e);
            entered(e.altKey);
          }
        }}
        onPointerLeave={domExited}
        onPointerMove={(event) => {
          if (nativePointer) return;
          followMark(event);
          openAtTop(event);
        }}
      >
        {draggable && (
          <div
            className="hud-grab-surface"
            aria-hidden="true"
            // The resting strip owns drag/click capture and hover for this child.
            style={{
              position: "absolute",
              ...grabBox,
              zIndex: 4,
              background: "transparent",
              cursor: dragVisual ? "grabbing" : "default",
            }}
            onClick={(event) => {
              const key = vendorUnderPointer(event) ?? hovered;
              if (key) pin(key);
            }}
          />
        )}
        {retracted ? (
          <EdgeTab
            edge={edge}
            vendors={vendors}
            fill={fill}
            outline={outline}
            light={light}
          />
        ) : slideFromRetract ? (
          <motion.div
            initial={edgeSlideInitial(edge)}
            animate={{ x: 0, y: 0, opacity: 1 }}
            transition={{ duration: 0.16, ease: "easeOut" }}
            style={{
              width: "100%",
              height: "100%",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
            }}
          >
            {children}
          </motion.div>
        ) : (
          children
        )}
      </div>
      {dragVisual && (
        <svg
          aria-hidden="true"
          className="hud-drag-outline"
          width={grab.frame.width}
          height={grab.frame.height}
          style={{
            ...position(grab.frame),
            zIndex: 5,
            overflow: "visible",
            pointerEvents: "none",
            filter: "drop-shadow(0 0 2px rgba(0,0,0,0.8))",
          }}
        >
          <rect
            x={grab.outline.x - grab.frame.x}
            y={grab.outline.y - grab.frame.y}
            width={grab.outline.width}
            height={grab.outline.height}
            rx={grab.radius}
            fill="rgba(255,255,255,0.025)"
            stroke="rgba(255,255,255,0.9)"
            strokeWidth={1.25}
            strokeDasharray="4 3"
          />
        </svg>
      )}
    </>
  );
}
