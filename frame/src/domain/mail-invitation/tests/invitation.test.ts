import { expect, test } from "vitest";
import { invitation } from "./fixtures";
import { canApply, canRespond, checkSender, oneLine, parseWhen, timeText } from "../checks";

const from = (...emails: string[]) => emails.map((email) => ({ name: null, email }));

test("a request or cancellation must come from its organizer, ignoring case", () => {
  for (const method of ["request", "cancel"] as const) {
    const inv = invitation({ method });
    expect(checkSender(inv, from("boss@example.com"))).toEqual({ ok: true });
    expect(checkSender(inv, from(" BOSS@EXAMPLE.COM "))).toEqual({ ok: true });
  }
});

test("a spoofed organizer is refused: another sender, a lookalike, or a mixed From", () => {
  const inv = invitation();
  expect(checkSender(inv, from("evil@example.net"))).toEqual({
    ok: false,
    reason: "organizer",
    named: "Boss@Example.com",
  });
  expect(checkSender(inv, from("boss@example.com.evil.net")).ok).toBe(false);
  expect(checkSender(inv, from("boss@examp1e.com")).ok).toBe(false);
  expect(checkSender(inv, from("boss@example.com", "evil@example.net")).ok).toBe(false);
  expect(checkSender(inv, [])).toMatchObject({ ok: false, reason: "no_sender" });
  expect(checkSender(invitation({ organizer: null }), from("boss@example.com"))).toMatchObject({
    ok: false,
    reason: "no_organizer",
  });
});

test("a reply must come from the attendee it speaks for, and from no one else", () => {
  const person = (email: string) => ({
    person: { email, name: null, partstat: "accepted" },
    role: "required",
    rsvp: false,
    is_you: false,
  });
  const reply = invitation({
    method: "reply",
    is_organizer: true,
    attendees: [person("guest@example.org")],
  });
  expect(checkSender(reply, from("Guest@Example.org"))).toEqual({ ok: true });
  expect(checkSender(reply, from("someone@example.org"))).toMatchObject({
    ok: false,
    reason: "attendee",
    named: "guest@example.org",
  });
  // A reply that speaks for several people cannot be tied to one sender.
  const many = invitation({
    method: "reply",
    attendees: [person("guest@example.org"), person("other@example.org")],
  });
  expect(checkSender(many, from("guest@example.org")).ok).toBe(false);
  expect(checkSender(invitation({ method: "reply", attendees: [] }), from("a@b.c")).ok).toBe(false);
  // A counter proposal is held to the same rule.
  expect(
    checkSender(
      invitation({ method: "counter", attendees: [person("guest@example.org")] }),
      from("evil@example.org"),
    ).ok,
  ).toBe(false);
});

test("answering and applying need a verified sender and the right kind of message", () => {
  const ok = { ok: true } as const;
  const bad = { ok: false, reason: "organizer", named: "x" } as const;
  expect(canRespond(invitation(), ok)).toBe(true);
  expect(canRespond(invitation(), bad)).toBe(false);
  expect(canRespond(invitation({ can_respond: false }), ok)).toBe(false);
  expect(canRespond(invitation({ is_organizer: true }), ok)).toBe(false);
  expect(canRespond(invitation({ method: "cancel" }), ok)).toBe(false);
  const stored = (freshness: "newer" | "same" | "older") => ({
    calendar_id: "c",
    read_only: false,
    status: "confirmed",
    partstat: null,
    freshness,
    same_organizer: true,
  });
  expect(canRespond(invitation({ existing: stored("newer") }), ok)).toBe(true);
  expect(canRespond(invitation({ existing: stored("older") }), ok)).toBe(false);

  expect(canApply(invitation({ method: "cancel" }), ok)).toBe(true);
  expect(canApply(invitation({ method: "cancel" }), bad)).toBe(false);
  expect(canApply(invitation({ method: "reply", is_organizer: true }), ok)).toBe(true);
  expect(canApply(invitation({ method: "reply", is_organizer: false }), ok)).toBe(false);
  expect(canApply(invitation({ method: "counter", is_organizer: true }), ok)).toBe(false);
  expect(canApply(invitation({ method: "request" }), ok)).toBe(false);
});

test("times are read in the viewer's zone, whole days end the day before their exclusive end", () => {
  const instant = parseWhen("2026-11-20T09:00:00Z")!;
  expect(instant.date.toISOString()).toBe("2026-11-20T09:00:00.000Z");
  expect(parseWhen("2026-11-20T09:00:00")!.date.getHours()).toBe(9);
  expect(parseWhen("2026-11-20")).toMatchObject({ dateOnly: true });
  expect(parseWhen("nonsense")).toBeNull();
  expect(parseWhen("2026-13-45T99:00:00Z")).toBeNull();
  expect(timeText({ start: "x", end: "y", all_day: false })).toBe("");
  const one = timeText({ start: "2026-11-20", end: "2026-11-21", all_day: true }, "en");
  expect(one).toContain("20");
  expect(one).not.toContain("–");
  expect(timeText({ start: "2026-11-20", end: "2026-11-23", all_day: true }, "en")).toContain("–");
  const timed = timeText(
    { start: "2026-11-20T09:00:00", end: "2026-11-20T10:30:00", all_day: false },
    "en",
  );
  expect(timed).toContain("–");
  expect(timed.match(/2026/gu)).toHaveLength(1);
});

test("text that goes into a reply is one bounded line", () => {
  expect(oneLine("a\r\nBcc: x@y.z", 50)).toBe("a Bcc: x@y.z");
  expect(oneLine("x".repeat(500), 200)).toHaveLength(200);
});
