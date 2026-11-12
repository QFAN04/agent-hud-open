import type { HUDConfig } from "../model";

export const SURFACE_BORDER_PX = 2;

export function isSurfaceBordered(
  mode: HUDConfig["mode"],
  detached: boolean,
  simplePeek: boolean,
): boolean {
  return (detached || mode === "simple") && !simplePeek;
}

export function peekBubbleHeight(
  vendor: { quotas: readonly unknown[]; balance: string | null },
  scale: number,
  bordered: boolean,
): number {
  const emptyFallback = !vendor.quotas.length && !vendor.balance ? 1 : 0;
  const rows =
    1 + vendor.quotas.length + (vendor.balance ? 1 : 0) + emptyFallback;
  return (20 + rows * 18) * scale + (bordered ? SURFACE_BORDER_PX : 0);
}

export function expandedPanelHeight(
  contentHeight: number,
  screenHeight: number,
  bordered: boolean,
): number {
  const borderAllowance = bordered ? SURFACE_BORDER_PX : 0;
  return Math.max(
    80,
    Math.min(contentHeight + borderAllowance, screenHeight - 80),
  );
}

export function bodyMaxHeight(
  frameHeight: number,
  padding: { top: number; bottom: number },
  bordered: boolean,
): number {
  return Math.max(
    40,
    frameHeight -
      (bordered ? SURFACE_BORDER_PX : 0) -
      padding.top -
      padding.bottom,
  );
}
