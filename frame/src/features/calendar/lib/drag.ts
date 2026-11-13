import { parseDayKey } from "$domain/calendar";

/** Pointer travel before a press on an event becomes a drag, in pixels. */
export const DRAG_SLOP = 4;

/** Snap step of the time grid, in minutes. */
export const SNAP_MINUTES = 15;

export const snap = (minutes: number, step = SNAP_MINUTES): number =>
  Math.round(minutes / step) * step;

/** The day (`data-day="YYYY-MM-DD"`) drawn under a point, if any. */
export function dayUnder(x: number, y: number): Date | null {
  const key = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-day]")?.dataset.day;
  return key ? parseDayKey(key) : null;
}
