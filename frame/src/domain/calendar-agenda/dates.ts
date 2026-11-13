import type { CalendarOccurrenceV1 } from "$shared/ipc/bindings";
import { linkId, linkOf } from "$domain/calendar-link";

/** Calendar math in the viewer's local zone. Days are added by calendar
 *  arithmetic (`new Date(y, m, d + n)`), never by 24-hour offsets, so a
 *  daylight-saving day of 23 or 25 hours keeps every grid aligned. */

const DAY_MS = 86_400_000;

export type CalendarView = "day" | "week" | "month" | "agenda";

/** Days listed by the agenda from its anchor. */
const AGENDA_DAYS = 30;

export const startOfDay = (date: Date): Date =>
  new Date(date.getFullYear(), date.getMonth(), date.getDate());

export const addDays = (date: Date, days: number): Date =>
  new Date(
    date.getFullYear(),
    date.getMonth(),
    date.getDate() + days,
    date.getHours(),
    date.getMinutes(),
    date.getSeconds(),
  );

/** Whole local days since 1970-01-01, exact across daylight-saving changes. */
export const dayNumber = (date: Date): number =>
  Math.floor(Date.UTC(date.getFullYear(), date.getMonth(), date.getDate()) / DAY_MS);

const two = (value: number) => String(value).padStart(2, "0");

export const dayKey = (date: Date): string =>
  `${String(date.getFullYear()).padStart(4, "0")}-${two(date.getMonth() + 1)}-${two(date.getDate())}`;

export const timeKey = (date: Date): string => `${two(date.getHours())}:${two(date.getMinutes())}`;

const DAY_KEY = /^(\d{4})-(\d{2})-(\d{2})$/u;
const FLOATING = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})(?::(\d{2}))?$/u;

const INSTANT = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?(?:Z|[+-]\d{2}:?\d{2})$/iu;

/** Local midnight of `YYYY-MM-DD`, or null when it is not a real date. */
export function parseDayKey(value: string): Date | null {
  const match = DAY_KEY.exec(value);
  if (!match) return null;
  const [year, month, day] = [Number(match[1]), Number(match[2]) - 1, Number(match[3])];
  const date = new Date(year, month, day);
  return date.getFullYear() === year && date.getMonth() === month && date.getDate() === day
    ? date
    : null;
}

/** An instant (`…Z` or with an offset), a floating wall-clock time read in the
 *  local zone, or an all-day date at local midnight. Null when malformed. */
export function parseCalendarTime(value: string): Date | null {
  const day = parseDayKey(value);
  if (day) return day;
  const floating = FLOATING.exec(value);
  if (floating) {
    const [year, month, date, hour, minute, second] = [
      Number(floating[1]),
      Number(floating[2]) - 1,
      Number(floating[3]),
      Number(floating[4]),
      Number(floating[5]),
      Number(floating[6] ?? 0),
    ];
    if (hour > 23 || minute > 59 || second > 59) return null;
    return new Date(year, month, date, hour, minute, second);
  }
  // Only a full RFC 3339 instant: `Date` would roll an impossible date over.
  if (!INSTANT.test(value)) return null;
  const instant = new Date(value);
  return Number.isNaN(instant.getTime()) ? null : instant;
}

/** First day of the week holding `date`; `firstDay` counts 0 for Sunday. */
export function startOfWeek(date: Date, firstDay: number): Date {
  const midnight = startOfDay(date);
  return addDays(midnight, -((midnight.getDay() - firstDay + 7) % 7));
}

/** The days of a month grid: whole weeks from the week of the 1st to the week
 *  of the last day, so five or six rows. */
export function monthGrid(anchor: Date, firstDay: number): Date[] {
  const first = new Date(anchor.getFullYear(), anchor.getMonth(), 1);
  const last = new Date(anchor.getFullYear(), anchor.getMonth() + 1, 0);
  const start = startOfWeek(first, firstDay);
  const weeks = Math.ceil((dayNumber(last) - dayNumber(start) + 1) / 7);
  return Array.from({ length: weeks * 7 }, (_, index) => addDays(start, index));
}

/** Local days the view shows for `anchor`. */
export function visibleDays(view: CalendarView, anchor: Date, firstDay: number): Date[] {
  const day = startOfDay(anchor);
  switch (view) {
    case "day":
      return [day];
    case "week": {
      const start = startOfWeek(day, firstDay);
      return Array.from({ length: 7 }, (_, index) => addDays(start, index));
    }
    case "month":
      return monthGrid(day, firstDay);
    case "agenda":
      return Array.from({ length: AGENDA_DAYS }, (_, index) => addDays(day, index));
  }
}

/** Half-open range of instants the view needs, as `calendar_events` takes it. */
export function rangeOf(
  view: CalendarView,
  anchor: Date,
  firstDay: number,
): { from: Date; to: Date; fromText: string; toText: string } {
  const days = visibleDays(view, anchor, firstDay);
  const from = days[0]!;
  const to = addDays(days[days.length - 1]!, 1);
  return { from, to, fromText: from.toISOString(), toText: to.toISOString() };
}

/** Moves the anchor one step: a day, a week, a month (clamped to its length)
 *  or the agenda's span. */
export function stepAnchor(view: CalendarView, anchor: Date, direction: 1 | -1): Date {
  const day = startOfDay(anchor);
  switch (view) {
    case "day":
      return addDays(day, direction);
    case "week":
      return addDays(day, 7 * direction);
    case "agenda":
      return addDays(day, AGENDA_DAYS * direction);
    case "month": {
      const target = new Date(day.getFullYear(), day.getMonth() + direction, 1);
      const length = new Date(target.getFullYear(), target.getMonth() + 1, 0).getDate();
      target.setDate(Math.min(day.getDate(), length));
      return target;
    }
  }
}

export const sameDay = (a: Date, b: Date): boolean => dayNumber(a) === dayNumber(b);

/** An occurrence with its times read as local `Date`s. */
export type PlacedEvent = {
  occurrence: CalendarOccurrenceV1;
  /** Calendar, event and instance: unique among the loaded occurrences. */
  id: string;
  start: Date;
  /** Exclusive for all-day events, which end at a local midnight. */
  end: Date;
  allDay: boolean;
};

const occurrenceId = (occurrence: CalendarOccurrenceV1): string => linkId(linkOf(occurrence));

/** Reads an occurrence's times; an unreadable one is dropped, not guessed. */
export function place(occurrence: CalendarOccurrenceV1): PlacedEvent | null {
  const start = parseCalendarTime(occurrence.start);
  let end = parseCalendarTime(occurrence.end);
  if (!start || !end) return null;
  if (occurrence.all_day) {
    const first = startOfDay(start);
    return {
      occurrence,
      id: occurrenceId(occurrence),
      start: first,
      end: end.getTime() > first.getTime() ? startOfDay(end) : addDays(first, 1),
      allDay: true,
    };
  }
  if (end.getTime() < start.getTime()) end = start;
  return { occurrence, id: occurrenceId(occurrence), start, end, allDay: false };
}

export const placeAll = (occurrences: readonly CalendarOccurrenceV1[]): PlacedEvent[] =>
  occurrences.flatMap((occurrence) => place(occurrence) ?? []);

const atMidnight = (date: Date) =>
  date.getHours() === 0 && date.getMinutes() === 0 && date.getSeconds() === 0;

/** First local day and the day after the last one an event touches. A timed
 *  event that ends exactly at midnight does not touch the next day. */
function daySpan(event: PlacedEvent): { first: number; after: number } {
  const first = dayNumber(event.start);
  if (event.allDay) return { first, after: Math.max(first + 1, dayNumber(event.end)) };
  if (event.end.getTime() <= event.start.getTime()) return { first, after: first + 1 };
  const last = atMidnight(event.end) ? dayNumber(event.end) : dayNumber(event.end) + 1;
  return { first, after: Math.max(first + 1, last) };
}

/** Whether the event is shown as a banner rather than at a time: all-day
 *  events and timed events lasting a day or more. */
export const isBanner = (event: PlacedEvent): boolean =>
  event.allDay || event.end.getTime() - event.start.getTime() >= DAY_MS;

export type Segment = {
  event: PlacedEvent;
  /** Column of the first day shown, from 0. */
  column: number;
  span: number;
  lane: number;
  /** Continues from an earlier row or day. */
  continuesBefore: boolean;
  /** Continues into a later row or day. */
  continuesAfter: boolean;
};

/** Places events as bars across a run of consecutive days (a week row, or the
 *  banner above a time grid), packing them into the fewest lanes. */
export function packSegments(events: readonly PlacedEvent[], days: readonly Date[]): Segment[] {
  if (days.length === 0) return [];
  const origin = dayNumber(days[0]!);
  const total = days.length;
  const segments: Omit<Segment, "lane">[] = [];
  for (const event of events) {
    const { first, after } = daySpan(event);
    const from = Math.max(first, origin);
    const to = Math.min(after, origin + total);
    if (from >= to) continue;
    segments.push({
      event,
      column: from - origin,
      span: to - from,
      continuesBefore: first < origin,
      continuesAfter: after > origin + total,
    });
  }
  segments.sort(
    (a, b) =>
      a.column - b.column ||
      b.span - a.span ||
      a.event.start.getTime() - b.event.start.getTime() ||
      a.event.id.localeCompare(b.event.id),
  );
  const laneEnds: number[] = [];
  return segments.map((segment) => {
    let lane = laneEnds.findIndex((end) => end <= segment.column);
    if (lane < 0) lane = laneEnds.length;
    laneEnds[lane] = segment.column + segment.span;
    return { ...segment, lane };
  });
}

export type TimedSegment = {
  event: PlacedEvent;
  /** Minutes after local midnight, wall-clock: a day is always 1440 of them. */
  top: number;
  bottom: number;
  column: number;
  columns: number;
  continuesBefore: boolean;
  continuesAfter: boolean;
};

const minuteOfDay = (date: Date) => date.getHours() * 60 + date.getMinutes();

/** Shortest drawn height of a timed event, in minutes. */
const MIN_MINUTES = 30;

/** Splits overlapping intervals into side-by-side columns. Intervals joined by
 *  a chain of overlaps share one cluster and so one column count. */
export function layoutOverlaps(
  intervals: readonly { top: number; bottom: number }[],
): { column: number; columns: number }[] {
  const order = intervals
    .map((interval, index) => ({ ...interval, index }))
    .sort((a, b) => a.top - b.top || b.bottom - a.bottom || a.index - b.index);
  const result = new Array<{ column: number; columns: number }>(intervals.length);
  let cluster: { index: number; column: number }[] = [];
  let clusterEnd = -Infinity;
  const columnEnds: number[] = [];
  const close = () => {
    for (const member of cluster)
      result[member.index] = { column: member.column, columns: columnEnds.length };
    cluster = [];
    columnEnds.length = 0;
    clusterEnd = -Infinity;
  };
  for (const item of order) {
    if (item.top >= clusterEnd) close();
    let column = columnEnds.findIndex((end) => end <= item.top);
    if (column < 0) column = columnEnds.length;
    columnEnds[column] = item.bottom;
    cluster.push({ index: item.index, column });
    clusterEnd = Math.max(clusterEnd, item.bottom);
  }
  close();
  return result;
}

/** Timed events as they stand on one day's column, clipped to the day. */
export function timedSegments(events: readonly PlacedEvent[], day: Date): TimedSegment[] {
  const number = dayNumber(day);
  const clipped = events.flatMap((event) => {
    if (isBanner(event)) return [];
    const { first, after } = daySpan(event);
    if (number < first || number >= after) return [];
    const startsToday = first === number;
    const endsToday = after === number + 1;
    const top = startsToday ? minuteOfDay(event.start) : 0;
    const instant = event.end.getTime() <= event.start.getTime();
    let end = 1440;
    if (instant) end = top + MIN_MINUTES;
    else if (endsToday && !atMidnight(event.end)) end = minuteOfDay(event.end);
    const bottom = Math.min(1440, Math.max(end, top + MIN_MINUTES));
    return [{ event, top, bottom, continuesBefore: !startsToday, continuesAfter: !endsToday }];
  });
  const layout = layoutOverlaps(clipped);
  return clipped.map((segment, index) => ({ ...segment, ...layout[index]! }));
}

/** Per-day lists for the agenda, in start order, skipping empty days. */
export function agendaDays(
  events: readonly PlacedEvent[],
  days: readonly Date[],
): { day: Date; events: PlacedEvent[] }[] {
  const sorted = [...events].sort(
    (a, b) =>
      Number(b.allDay) - Number(a.allDay) ||
      a.start.getTime() - b.start.getTime() ||
      a.id.localeCompare(b.id),
  );
  return days
    .map((day) => {
      const number = dayNumber(day);
      return {
        day,
        events: sorted.filter((event) => {
          const { first, after } = daySpan(event);
          return number >= first && number < after;
        }),
      };
    })
    .filter((entry) => entry.events.length > 0);
}
