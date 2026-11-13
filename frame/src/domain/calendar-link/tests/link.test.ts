import { describe, expect, it, vi } from "vitest";
import type { CalendarOccurrenceV1 } from "../../../shared/ipc/bindings";
import { eventUrl, linkOf, parseEventUrl } from "../index";

vi.mock("$domain/surface", () => ({ surface: { open: vi.fn() } }));

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

describe("event links", () => {
  it("round-trips through the address a task stores", () => {
    const link = linkOf(occurrence({ recurrence_id: "2026-10-14T09:30:00", start: "x" }));
    expect(link).toEqual({
      calendarId: "work/1",
      uid: "abc@example.com",
      at: "2026-10-14T09:30:00",
    });
    expect(parseEventUrl(eventUrl(link))).toEqual(link);
  });

  it("is a web address the browser's gate would accept, never one that resolves", () => {
    const url = new URL(eventUrl(linkOf(occurrence())));
    expect(url.protocol).toBe("https:");
    expect(url.hostname.endsWith(".invalid")).toBe(true);
  });

  it("recognises no other address", () => {
    expect(parseEventUrl("https://example.com/event?calendar=a&uid=b&at=c")).toBeNull();
    expect(parseEventUrl("http://calendar.zephium.invalid/event?calendar=a&uid=b&at=c")).toBeNull();
    expect(
      parseEventUrl("https://calendar.zephium.invalid/other?calendar=a&uid=b&at=c"),
    ).toBeNull();
    expect(parseEventUrl("https://calendar.zephium.invalid/event?calendar=a&uid=b")).toBeNull();
    expect(parseEventUrl("not a url")).toBeNull();
  });
});
