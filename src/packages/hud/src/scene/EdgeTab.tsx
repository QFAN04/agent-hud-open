import { horizontal } from "../geometry";
import { statusColor, type HUDEdge, type HUDVendor } from "../model";

/** Slim handle shown along the screen edge while a Simple dock is retracted. */
export function EdgeTab({
  edge,
  vendors,
  fill,
  outline,
  light,
}: {
  edge: HUDEdge;
  vendors: readonly HUDVendor[];
  fill: string;
  outline: string;
  light: boolean;
}) {
  const horiz = horizontal(edge);
  const dots = vendors.slice(0, 5);
  const radius =
    edge === "left"
      ? "0 5px 5px 0"
      : edge === "right"
        ? "5px 0 0 5px"
        : edge === "top"
          ? "0 0 5px 5px"
          : "5px 5px 0 0";
  const idleDot = light ? "rgba(0,0,0,0.28)" : "rgba(255,255,255,0.38)";
  return (
    <div
      className="hud-edge-tab"
      aria-label="Agent HUD edge handle"
      style={{
        width: "100%",
        height: "100%",
        display: "flex",
        flexDirection: horiz ? "row" : "column",
        alignItems: "center",
        justifyContent: "center",
        gap: 3,
        borderRadius: radius,
        background: fill,
        boxShadow: `inset 0 0 0 1px ${outline}`,
      }}
    >
      {dots.length > 0 ? (
        dots.map((vendor) => (
          <span
            key={vendor.key}
            style={{
              width: 3,
              height: 3,
              borderRadius: "50%",
              background:
                vendor.used != null ? statusColor(vendor.level) : idleDot,
              opacity: vendor.used != null ? 0.92 : 0.7,
            }}
          />
        ))
      ) : (
        <span
          style={{
            width: horiz ? 14 : 3,
            height: horiz ? 3 : 14,
            borderRadius: 2,
            background: idleDot,
          }}
        />
      )}
    </div>
  );
}
