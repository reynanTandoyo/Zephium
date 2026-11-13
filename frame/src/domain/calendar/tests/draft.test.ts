import { beforeAll, describe, expect, it } from "vitest";
import type { CalendarOccurrenceV1 } from "../../../shared/ipc/bindings";
import { place } from "../dates";
import {
  editPolicy,
  formForSlot,
  formFromEvent,
  formToDraft,
  movedForm,
  wallClock,
} from "../draft";
import type { EventForm } from "../draft";
import { meetingLink } from "../meeting";

beforeAll(() => {
  process.env.TZ = "Europe/Berlin";
});

const base: EventForm = {
  calendarId: "cal",
  summary: "  Review  ",
  allDay: false,
  startDate: "2026-10-07",
  startTime: "09:00",
  endDate: "2026-10-07",
  endTime: "10:30",
  location: " Room 4 ",
  description: "Notes",
  recurrence: "none",
  reminder: "15",
  url: null,
  moreReminders: [],
  zone: null,
};

const occurrence = (patch: Partial<CalendarOccurrenceV1> = {}): CalendarOccurrenceV1 => ({
  calendar_id: "cal",
  uid: "u1",
  recurrence_id: null,
  summary: "Review",
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

describe("event drafts", () => {
  it("writes timed events as wall-clock times in the zone", () => {
    expect(formToDraft(base, "Europe/Berlin")).toEqual({
      ok: true,
      draft: {
        summary: "Review",
        description: "Notes",
        location: "Room 4",
        start: "2026-10-07T09:00:00",
        end: "2026-10-07T10:30:00",
        all_day: false,
        tzid: "Europe/Berlin",
        rrule: null,
        reminders: [15],
        url: null,
      },
    });
  });

  it("keeps the event's own zone and its further reminders", () => {
    const result = formToDraft(
      { ...base, zone: "America/New_York", moreReminders: [60] },
      "Europe/Berlin",
    );
    expect(result.ok && [result.draft.start, result.draft.tzid, result.draft.reminders]).toEqual([
      "2026-10-07T03:00:00",
      "America/New_York",
      [15, 60],
    ]);
    const odd = formToDraft({ ...base, zone: "Custom/Nowhere" }, "Europe/Berlin");
    expect(odd.ok && odd.draft.tzid).toBe("Europe/Berlin");
  });

  it("converts to another zone", () => {
    const result = formToDraft(base, "America/New_York");
    expect(result.ok && result.draft.start).toBe("2026-10-07T03:00:00");
  });

  it("keeps the wall clock on the daylight-saving day", () => {
    expect(wallClock(new Date(2026, 2, 29, 3, 30), "Europe/Berlin")).toBe("2026-03-29T03:30:00");
  });

  it("writes an all-day event with an exclusive end", () => {
    const result = formToDraft({ ...base, allDay: true, endDate: "2026-10-09" }, "Europe/Berlin");
    expect(result.ok && [result.draft.start, result.draft.end, result.draft.tzid]).toEqual([
      "2026-10-07",
      "2026-10-10",
      null,
    ]);
  });

  it("maps recurrence to an RRULE", () => {
    const rule = (recurrence: EventForm["recurrence"]) => {
      const result = formToDraft({ ...base, recurrence });
      return result.ok ? result.draft.rrule : "error";
    };
    expect([rule("none"), rule("daily"), rule("weekly"), rule("monthly"), rule("yearly")]).toEqual([
      null,
      "FREQ=DAILY",
      "FREQ=WEEKLY",
      "FREQ=MONTHLY",
      "FREQ=YEARLY",
    ]);
  });

  it("refuses an end before the start and a bad reminder", () => {
    expect(formToDraft({ ...base, endTime: "08:00" })).toEqual({ ok: false, error: "range" });
    expect(formToDraft({ ...base, startDate: "2026-02-31" })).toEqual({
      ok: false,
      error: "range",
    });
    expect(formToDraft({ ...base, allDay: true, endDate: "2026-10-06" })).toEqual({
      ok: false,
      error: "range",
    });
    expect(formToDraft({ ...base, reminder: "-5" })).toEqual({ ok: false, error: "reminder" });
    expect(formToDraft({ ...base, reminder: "1.5" })).toEqual({ ok: false, error: "reminder" });
    expect(formToDraft({ ...base, reminder: "" })).toMatchObject({
      ok: true,
      draft: { reminders: [] },
    });
  });

  it("round-trips an existing all-day event with an inclusive end date", () => {
    const event = place(occurrence({ all_day: true, start: "2026-10-07", end: "2026-10-09" }))!;
    const form = formFromEvent(event);
    expect([form.startDate, form.endDate]).toEqual(["2026-10-07", "2026-10-08"]);
    const result = formToDraft(form);
    expect(result.ok && result.draft.end).toBe("2026-10-09");
  });

  it("builds a slot form and moves an event keeping its length", () => {
    const slot = formForSlot("cal", new Date(2026, 9, 7, 14), new Date(2026, 9, 7, 15));
    expect([slot.startTime, slot.endTime, slot.allDay]).toEqual(["14:00", "15:00", false]);
    const event = place(occurrence({ start: "2026-10-07T23:00:00", end: "2026-10-08T00:30:00" }))!;
    const moved = movedForm(event, new Date(2026, 9, 9, 8, 15));
    expect([moved.startDate, moved.startTime, moved.endDate, moved.endTime]).toEqual([
      "2026-10-09",
      "08:15",
      "2026-10-09",
      "09:45",
    ]);
    const allDay = place(occurrence({ all_day: true, start: "2026-10-07", end: "2026-10-09" }))!;
    const shifted = movedForm(allDay, new Date(2026, 9, 10));
    expect([shifted.startDate, shifted.endDate]).toEqual(["2026-10-10", "2026-10-11"]);
  });
});

describe("edit policy", () => {
  it("allows plain events, locks series and honours read-only calendars", () => {
    expect(editPolicy(occurrence())).toBe("edit");
    expect(editPolicy(occurrence({ recurring: true }))).toBe("series-locked");
    expect(editPolicy(occurrence({ read_only: true, recurring: true }))).toBe("read-only");
  });
});

describe("meeting links", () => {
  const link = (patch: Partial<CalendarOccurrenceV1>) => meetingLink(occurrence(patch));

  it("finds Teams, Meet and Zoom links in the url, location or notes", () => {
    expect(link({ url: "https://teams.microsoft.com/l/meetup-join/19%3a123/0?x=1" })?.service).toBe(
      "teams",
    );
    expect(link({ location: "https://meet.google.com/abc-defg-hij" })?.service).toBe("meet");
    expect(link({ description: "Join: https://us02web.zoom.us/j/12345?pwd=abc." })?.url).toBe(
      "https://us02web.zoom.us/j/12345?pwd=abc",
    );
  });

  it("ignores other addresses, lookalike hosts and insecure links", () => {
    expect(link({ location: "https://example.com/j/1" })).toBeNull();
    expect(link({ location: "https://zoom.us.evil.example/j/1" })).toBeNull();
    expect(link({ location: "http://meet.google.com/abc-defg-hij" })).toBeNull();
    expect(link({ location: "https://teams.microsoft.com/" })).toBeNull();
    expect(link({})).toBeNull();
  });
});
