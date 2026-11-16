import { expect, test } from "vitest";
import type { CalendarOccurrenceV1 } from "$shared/ipc/bindings";
import { placeAll } from "$domain/calendar";
import { withUniqueIds } from "../lib/keys";

const occurrence = (summary: string, uid = "u1"): CalendarOccurrenceV1 =>
  ({
    calendar_id: "c1",
    uid,
    recurrence_id: null,
    summary,
    description: "",
    location: "",
    start: "2026-01-12T09:00:00Z",
    end: "2026-01-12T10:00:00Z",
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
    series_start: null,
    series_end: null,
    series_tzid: null,
    rrule: null,
    recurrence: null,
  }) as CalendarOccurrenceV1;

test("two resources with the same UID and start get different ids, the rest keep theirs", () => {
  const placed = placeAll([
    occurrence("A"),
    occurrence("B"),
    occurrence("C", "u2"),
    occurrence("D"),
  ]);
  expect(new Set(placed.map((event) => event.id)).size).toBe(2);
  const unique = withUniqueIds(placed);
  expect(new Set(unique.map((event) => event.id)).size).toBe(4);
  expect(unique[0]!.id).toBe(placed[0]!.id);
  expect(unique[1]!.id).toBe(`${placed[0]!.id}~2`);
  expect(unique[2]!.id).toBe(placed[2]!.id);
  expect(unique[3]!.id).toBe(`${placed[0]!.id}~3`);
  // The events themselves are untouched, and the result is stable.
  expect(unique.map((event) => event.occurrence.summary)).toEqual(["A", "B", "C", "D"]);
  expect(withUniqueIds(placed).map((event) => event.id)).toEqual(unique.map((event) => event.id));
});

test("distinct ids stay as they are", () => {
  const placed = placeAll([occurrence("A", "u1"), occurrence("B", "u2")]);
  expect(withUniqueIds(placed)).toEqual(placed);
  expect(withUniqueIds([])).toEqual([]);
});
