import { describe, expect, it } from "vitest";
import type { CalendarOccurrenceV1 } from "../../../shared/ipc/bindings";
import { linkId, linkOf, parseEventUrl } from "$domain/calendar-link";
import { place } from "$domain/calendar-agenda";
import { taskInputOf } from "../task";

const occurrence = (patch: Partial<CalendarOccurrenceV1> = {}): CalendarOccurrenceV1 => ({
  calendar_id: "work/1",
  uid: "abc@example.com",
  recurrence_id: null,
  summary: "  Planning  ",
  description: "",
  location: "",
  start: "2026-10-07T09:30:00",
  end: "2026-10-07T10:30:00",
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

describe("event keys", () => {
  it("name the same key the views give the occurrence", () => {
    const single = occurrence();
    expect(linkId(linkOf(single))).toBe(place(single)!.id);
    const repeat = occurrence({ recurring: true, recurrence_id: "2026-10-14T09:30:00" });
    expect(linkId(linkOf(repeat))).toBe(place(repeat)!.id);
  });
});

describe("a task for an event", () => {
  it("is due when the event starts and links back to it", () => {
    const input = taskInputOf(place(occurrence())!);
    expect(input).toMatchObject({
      title: "Planning",
      dueDate: "2026-10-07",
      dueTime: "09:30",
      context: { title: "Planning" },
    });
    expect(parseEventUrl(input.context!.url)).toEqual(linkOf(occurrence()));
  });

  it("is due on the day only for an all-day event", () => {
    const input = taskInputOf(
      place(occurrence({ all_day: true, start: "2026-10-07", end: "2026-10-08" }))!,
    );
    expect([input.dueDate, input.dueTime]).toEqual(["2026-10-07", null]);
  });

  it("names an untitled event", () => {
    expect(taskInputOf(place(occurrence({ summary: "  " }))!).title).toBe("(No title)");
  });
});
