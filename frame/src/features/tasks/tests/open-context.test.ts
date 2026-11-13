import { afterEach, expect, test, vi } from "vitest";

const host = vi.hoisted(() => ({ open: vi.fn() }));
vi.mock("$domain/surface", () => ({ surface: { open: host.open } }));

const { eventOpen, eventUrl } = await import("$domain/calendar-link");
const { openContext } = await import("../lib/open-context");
const { hostOf } = await import("../lib/task-sections");

const link = { calendarId: "work", uid: "u1", at: "2026-10-07T09:00:00" };

afterEach(() => {
  eventOpen.clear();
  vi.clearAllMocks();
});

test("a task made from an event opens Calendar on it and leaves the browser alone", () => {
  const page = vi.fn();
  openContext(eventUrl(link), page);
  expect(page).not.toHaveBeenCalled();
  expect(host.open).toHaveBeenCalledWith("calendar");
  expect(eventOpen.pending()).toEqual(link);
});

test("any other link opens as a page", () => {
  const page = vi.fn();
  openContext("https://example.com/docs", page);
  expect(page).toHaveBeenCalledWith("https://example.com/docs");
  expect(host.open).not.toHaveBeenCalled();
  expect(eventOpen.pending()).toBeNull();
});

test("the row names the calendar, not the reserved host that carries the link", () => {
  expect(hostOf(eventUrl(link))).toBe("calendar");
  expect(hostOf("https://www.example.com/docs")).toBe("example.com");
});
