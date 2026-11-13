import { beforeAll, describe, expect, it } from "vitest";
import type { CalendarOccurrenceV1 } from "../../../shared/ipc/bindings";
import {
  addDays,
  agendaDays,
  dayKey,
  dayNumber,
  layoutOverlaps,
  monthGrid,
  packSegments,
  parseCalendarTime,
  placeAll,
  rangeOf,
  startOfWeek,
  stepAnchor,
  timedSegments,
  visibleDays,
} from "../dates";

beforeAll(() => {
  // Daylight saving in Berlin: 2026-03-29 has 23 hours, 2026-10-25 has 25.
  process.env.TZ = "Europe/Berlin";
});

let counter = 0;
const occurrence = (patch: Partial<CalendarOccurrenceV1>): CalendarOccurrenceV1 => ({
  calendar_id: "cal",
  uid: `uid-${counter++}`,
  recurrence_id: null,
  summary: "Event",
  description: "",
  location: "",
  start: "2026-10-07T09:00:00",
  end: "2026-10-07T10:00:00",
  all_day: false,
  tzid: null,
  color: null,
  status: "confirmed",
  recurring: false,
  read_only: false,
  url: null,
  organizer: null,
  attendees: [],
  reminders: [],
  ...patch,
});

describe("week starts", () => {
  it("starts on Monday, Sunday or Saturday as asked", () => {
    const wednesday = new Date(2026, 9, 7); // 2026-10-07
    expect(dayKey(startOfWeek(wednesday, 1))).toBe("2026-10-05");
    expect(dayKey(startOfWeek(wednesday, 0))).toBe("2026-10-04");
    expect(dayKey(startOfWeek(wednesday, 6))).toBe("2026-10-03");
  });

  it("keeps the date when it already starts the week", () => {
    expect(dayKey(startOfWeek(new Date(2026, 9, 5, 18), 1))).toBe("2026-10-05");
  });

  it("crosses a month and a year boundary", () => {
    expect(dayKey(startOfWeek(new Date(2026, 0, 1), 1))).toBe("2025-12-29");
  });
});

describe("month grid", () => {
  it("covers whole weeks around the month", () => {
    const grid = monthGrid(new Date(2026, 9, 15), 1);
    expect(grid).toHaveLength(35);
    expect(dayKey(grid[0]!)).toBe("2026-09-28");
    expect(dayKey(grid[34]!)).toBe("2026-11-01");
  });

  it("uses five or six rows as the month needs", () => {
    expect(monthGrid(new Date(2026, 2, 10), 0)).toHaveLength(35); // March 2026 starts on Sunday
    expect(monthGrid(new Date(2026, 7, 10), 0)).toHaveLength(42); // August 2026 starts on Saturday
  });

  it("stays on consecutive midnights through the spring daylight-saving change", () => {
    const grid = monthGrid(new Date(2026, 2, 29), 1);
    for (let index = 1; index < grid.length; index += 1)
      expect(dayNumber(grid[index]!) - dayNumber(grid[index - 1]!)).toBe(1);
    expect(grid.every((day) => day.getHours() === 0)).toBe(true);
  });
});

describe("views and ranges", () => {
  it("lists the days of each view", () => {
    const anchor = new Date(2026, 9, 7);
    expect(visibleDays("day", anchor, 1)).toHaveLength(1);
    expect(visibleDays("week", anchor, 1).map(dayKey)).toEqual([
      "2026-10-05",
      "2026-10-06",
      "2026-10-07",
      "2026-10-08",
      "2026-10-09",
      "2026-10-10",
      "2026-10-11",
    ]);
    expect(visibleDays("agenda", anchor, 1)).toHaveLength(30);
  });

  it("asks for the half-open range of local midnights as instants", () => {
    const range = rangeOf("week", new Date(2026, 9, 7), 1);
    expect(range.fromText).toBe("2026-10-04T22:00:00.000Z");
    expect(range.toText).toBe("2026-10-11T22:00:00.000Z");
  });

  it("measures a week across the autumn change from midnight to midnight", () => {
    const range = rangeOf("week", new Date(2026, 9, 25), 1);
    expect((range.to.getTime() - range.from.getTime()) / 3_600_000).toBe(7 * 24 + 1);
  });

  it("steps by view and clamps months", () => {
    expect(dayKey(stepAnchor("day", new Date(2026, 2, 28), 1))).toBe("2026-03-29");
    expect(dayKey(stepAnchor("week", new Date(2026, 9, 25), -1))).toBe("2026-10-18");
    expect(dayKey(stepAnchor("month", new Date(2026, 0, 31), 1))).toBe("2026-02-28");
    expect(dayKey(stepAnchor("month", new Date(2026, 0, 15), -1))).toBe("2025-12-15");
    expect(dayKey(stepAnchor("agenda", new Date(2026, 0, 1), 1))).toBe("2026-01-31");
  });

  it("adds days by the calendar, keeping the hour across daylight saving", () => {
    expect(addDays(new Date(2026, 2, 28, 9), 1).getHours()).toBe(9);
    expect(addDays(new Date(2026, 9, 24, 9), 2).getHours()).toBe(9);
  });
});

describe("reading times", () => {
  it("reads instants, floating times and dates", () => {
    expect(parseCalendarTime("2026-10-07T07:00:00Z")?.getHours()).toBe(9); // CEST
    expect(parseCalendarTime("2026-10-07T09:30:00")?.getMinutes()).toBe(30);
    expect(dayKey(parseCalendarTime("2026-10-07")!)).toBe("2026-10-07");
  });

  it("refuses malformed and impossible values", () => {
    expect(parseCalendarTime("2026-02-30")).toBeNull();
    expect(parseCalendarTime("2026-10-07T25:00:00")).toBeNull();
    expect(parseCalendarTime("tomorrow")).toBeNull();
    expect(placeAll([occurrence({ start: "nope" })])).toEqual([]);
  });
});

describe("placing events on days", () => {
  const week = visibleDays("week", new Date(2026, 9, 7), 1);

  it("places an all-day event on its day only (end is exclusive)", () => {
    const events = placeAll([
      occurrence({ all_day: true, start: "2026-10-07", end: "2026-10-08" }),
    ]);
    expect(packSegments(events, week)[0]).toMatchObject({ column: 2, span: 1, lane: 0 });
  });

  it("spans a multi-day all-day event and marks a continuation", () => {
    const events = placeAll([
      occurrence({ all_day: true, start: "2026-10-09", end: "2026-10-14" }),
    ]);
    expect(packSegments(events, week)[0]).toMatchObject({
      column: 4,
      span: 3,
      continuesAfter: true,
      continuesBefore: false,
    });
    const next = packSegments(events, visibleDays("week", new Date(2026, 9, 14), 1))[0]!;
    expect(next).toMatchObject({
      column: 0,
      span: 2,
      continuesBefore: true,
      continuesAfter: false,
    });
  });

  it("treats a missing end as a single day", () => {
    const events = placeAll([
      occurrence({ all_day: true, start: "2026-10-07", end: "2026-10-07" }),
    ]);
    expect(packSegments(events, week)[0]).toMatchObject({ span: 1 });
  });

  it("does not touch the next day when a timed event ends at midnight", () => {
    const events = placeAll([
      occurrence({ start: "2026-10-07T22:00:00", end: "2026-10-08T00:00:00" }),
    ]);
    expect(packSegments(events, week)[0]).toMatchObject({ column: 2, span: 1 });
  });

  it("runs an overnight timed event across both days", () => {
    const events = placeAll([
      occurrence({ start: "2026-10-07T22:00:00", end: "2026-10-08T02:00:00" }),
    ]);
    expect(packSegments(events, week)[0]).toMatchObject({ column: 2, span: 2 });
    const first = timedSegments(events, week[2]!)[0]!;
    const second = timedSegments(events, week[3]!)[0]!;
    expect([first.top, first.bottom, first.continuesAfter]).toEqual([1320, 1440, true]);
    expect([second.top, second.bottom, second.continuesBefore]).toEqual([0, 120, true]);
  });

  it("shows events lasting a day or more as banners, not in the time grid", () => {
    const events = placeAll([
      occurrence({ start: "2026-10-07T09:00:00", end: "2026-10-08T09:00:00" }),
    ]);
    expect(timedSegments(events, week[2]!)).toEqual([]);
  });

  it("packs overlapping bars into lanes and reuses a freed lane", () => {
    const events = placeAll([
      occurrence({ all_day: true, start: "2026-10-05", end: "2026-10-08" }),
      occurrence({ all_day: true, start: "2026-10-06", end: "2026-10-07" }),
      occurrence({ all_day: true, start: "2026-10-08", end: "2026-10-09" }),
    ]);
    const lanes = packSegments(events, week).map((segment) => [segment.column, segment.lane]);
    expect(lanes).toEqual([
      [0, 0],
      [1, 1],
      [3, 0],
    ]);
  });

  it("places events on the spring and autumn daylight-saving days", () => {
    const spring = visibleDays("day", new Date(2026, 2, 29), 1);
    const events = placeAll([
      occurrence({ start: "2026-03-29T00:30:00Z", end: "2026-03-29T01:30:00Z" }),
    ]); // 01:30 to 03:30 local
    const [segment] = timedSegments(events, spring[0]!);
    expect(segment!.top).toBe(90);
    expect(segment!.bottom).toBe(210);
    const autumn = visibleDays("day", new Date(2026, 9, 25), 1);
    const late = placeAll([
      occurrence({ start: "2026-10-25T22:00:00Z", end: "2026-10-25T23:00:00Z" }),
    ]);
    // 23:00 CET to midnight: the repeated 02:00 to 03:00 hour does not shift the grid.
    const [evening] = timedSegments(late, autumn[0]!);
    expect([evening!.top, evening!.bottom]).toEqual([1380, 1440]);
  });

  it("lists the agenda day by day, all-day first, skipping empty days", () => {
    const days = visibleDays("week", new Date(2026, 9, 7), 1);
    const events = placeAll([
      occurrence({ start: "2026-10-07T15:00:00", end: "2026-10-07T16:00:00", summary: "b" }),
      occurrence({ start: "2026-10-07T09:00:00", end: "2026-10-07T10:00:00", summary: "a" }),
      occurrence({ all_day: true, start: "2026-10-07", end: "2026-10-09", summary: "all" }),
    ]);
    const agenda = agendaDays(events, days);
    expect(agenda.map((entry) => dayKey(entry.day))).toEqual(["2026-10-07", "2026-10-08"]);
    expect(agenda[0]!.events.map((entry) => entry.occurrence.summary)).toEqual(["all", "a", "b"]);
    expect(agenda[1]!.events.map((entry) => entry.occurrence.summary)).toEqual(["all"]);
  });
});

describe("overlap layout", () => {
  it("gives separate events the full width", () => {
    expect(
      layoutOverlaps([
        { top: 0, bottom: 60 },
        { top: 60, bottom: 120 },
      ]),
    ).toEqual([
      { column: 0, columns: 1 },
      { column: 0, columns: 1 },
    ]);
  });

  it("puts overlapping events side by side", () => {
    expect(
      layoutOverlaps([
        { top: 0, bottom: 90 },
        { top: 30, bottom: 120 },
      ]),
    ).toEqual([
      { column: 0, columns: 2 },
      { column: 1, columns: 2 },
    ]);
  });

  it("shares one column count across a chain and reuses freed columns", () => {
    const result = layoutOverlaps([
      { top: 0, bottom: 60 },
      { top: 30, bottom: 120 },
      { top: 70, bottom: 130 },
    ]);
    expect(result).toEqual([
      { column: 0, columns: 2 },
      { column: 1, columns: 2 },
      { column: 0, columns: 2 },
    ]);
  });

  it("widens to three columns for three simultaneous events", () => {
    const result = layoutOverlaps([
      { top: 0, bottom: 60 },
      { top: 0, bottom: 60 },
      { top: 0, bottom: 60 },
    ]);
    expect(result.map((entry) => entry.columns)).toEqual([3, 3, 3]);
    expect(result.map((entry) => entry.column).sort()).toEqual([0, 1, 2]);
  });

  it("lays out a day with a minimum event height", () => {
    const events = placeAll([
      occurrence({ start: "2026-10-07T09:00:00", end: "2026-10-07T09:00:00" }),
      occurrence({ start: "2026-10-07T09:10:00", end: "2026-10-07T09:20:00" }),
    ]);
    const segments = timedSegments(events, new Date(2026, 9, 7));
    expect(segments.map((entry) => entry.columns)).toEqual([2, 2]);
    expect(segments[0]!.bottom - segments[0]!.top).toBe(30);
  });
});
