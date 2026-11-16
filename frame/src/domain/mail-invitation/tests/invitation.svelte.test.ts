import { afterEach, expect, test, vi } from "vitest";
import { invitation } from "./fixtures";

const host = vi.hoisted(() => ({
  bytes: vi.fn(),
  parse: vi.fn(),
  accounts: vi.fn(),
  calendars: vi.fn(),
  respond: vi.fn(),
  applyCancel: vi.fn(),
  applyReply: vi.fn(),
  send: vi.fn(),
}));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    mailInvitationBytes: host.bytes,
    calendarInvitationParse: host.parse,
    calendarAccounts: host.accounts,
    calendarCalendars: host.calendars,
    calendarInvitationRespond: host.respond,
    calendarInvitationApplyCancel: host.applyCancel,
    calendarInvitationApplyReply: host.applyReply,
    mailSend: host.send,
  });
});

const { MailInvitation, replyWords } = await import("../store.svelte");

const ok = <T>(data: T) => Promise.resolve({ status: "ok" as const, data });
const err = (error: string) => Promise.resolve({ status: "error" as const, error });

afterEach(() => {
  for (const mock of Object.values(host)) mock.mockReset();
});

const source = (over = {}) => ({
  account: "acc-1",
  folder: "INBOX",
  uid: 7,
  part: 2,
  from: [{ name: "Boss", email: "boss@example.com" }],
  messageId: "m7@example.com",
  own: ["me@example.com"],
  ownName: "Me",
  ...over,
});

const itip = (over = {}) => ({
  to: "boss@example.com",
  subject: "Accepted: Planning",
  body_text: "English only",
  ics: "BEGIN:VCALENDAR\r\nMETHOD:REPLY\r\nEND:VCALENDAR\r\n",
  content_type: "text/calendar; method=REPLY; charset=UTF-8",
  calendar_action: "created",
  ...over,
});

function calendars(extra: unknown[] = []) {
  host.accounts.mockReturnValue(
    ok([
      { id: "ac-dav", kind: "caldav", name: "Work", server_url: "", username: null },
      { id: "ac-sub", kind: "subscription", name: "Feed", server_url: "", username: null },
      ...extra,
    ]),
  );
  host.calendars.mockReturnValue(
    ok([
      {
        id: "cal-ro",
        account_id: "ac-dav",
        url: "",
        name: "Shared",
        color: null,
        read_only: true,
        visible: true,
      },
      {
        id: "cal-1",
        account_id: "ac-dav",
        url: "",
        name: "Mine",
        color: "#123456",
        read_only: false,
        visible: true,
      },
      {
        id: "cal-feed",
        account_id: "ac-sub",
        url: "",
        name: "Holidays",
        color: null,
        read_only: false,
        visible: true,
      },
      {
        id: "cal-g",
        account_id: "ac-g",
        url: "",
        name: "Google",
        color: null,
        read_only: false,
        visible: true,
      },
    ]),
  );
}

async function loaded(inv = invitation(), from = source()) {
  host.bytes.mockReturnValue(ok("QkVHSU4="));
  host.parse.mockReturnValue(ok(inv));
  const card = new MailInvitation(from);
  await card.load();
  return card;
}

test("loading reads the part, parses it with the account's own address and lists only writable CalDAV calendars", async () => {
  calendars([{ id: "ac-g", kind: "google", name: "G", server_url: "", username: null }]);
  const card = await loaded();
  expect(host.bytes).toHaveBeenCalledWith("acc-1", "INBOX", 7, 2);
  expect(host.parse).toHaveBeenCalledWith("QkVHSU4=", ["me@example.com"]);
  expect(card.state).toBe("ready");
  expect(card.calendars.map((c) => c.id)).toEqual(["cal-1"]);
  expect(card.chosen).toBe("cal-1");
  expect(card.providerCalendars).toBe(true);
  expect(card.mayRespond).toBe(true);
  card.dispose();
});

test("a part that cannot be read or parsed leaves a failed card", async () => {
  host.bytes.mockReturnValue(err("invalid_input"));
  const a = new MailInvitation(source());
  await a.load();
  expect(a.state).toBe("failed");
  host.bytes.mockReturnValue(ok("AA=="));
  host.parse.mockReturnValue(err("invalid_input"));
  const b = new MailInvitation(source());
  await b.load();
  expect(b.state).toBe("failed");
  expect(host.accounts).not.toHaveBeenCalled();
});

test("accepting answers in the calendar first, then sends a REPLY alternative from the same account", async () => {
  calendars();
  host.respond.mockReturnValue(ok(itip()));
  host.send.mockReturnValue(ok({ sent: true, saved_copy: true }));
  const card = await loaded();
  expect(host.send).not.toHaveBeenCalled();
  expect(await card.respond("accept")).toBe(true);
  expect(host.respond).toHaveBeenCalledWith("inv-1", "accept", "cal-1");
  const [account, draft] = host.send.mock.calls[0]!;
  expect(account).toBe("acc-1");
  expect(draft).toMatchObject({
    to: [{ name: "Boss", email: "boss@example.com" }],
    subject: "Accepted: Planning",
    html: null,
    attachments: [],
    in_reply_to: "m7@example.com",
    references: ["m7@example.com"],
    calendar: { method: "REPLY", ics: itip().ics },
  });
  // The text is ours, in the viewer's language, not the calendar crate's English.
  expect(draft.text).toContain("Planning");
  expect(draft.text).not.toBe("English only");
  expect(card.answered).toBe("accept");
  expect(card.send).toBe("sent");
  card.dispose();
});

test("declining needs no calendar and still asks the organizer's address only", async () => {
  host.accounts.mockReturnValue(ok([]));
  host.calendars.mockReturnValue(ok([]));
  host.respond.mockReturnValue(ok(itip({ subject: "Declined: Planning" })));
  host.send.mockReturnValue(ok({ sent: true, saved_copy: false }));
  const card = await loaded();
  expect(card.canStore).toBe(false);
  expect(await card.respond("accept")).toBe(false);
  expect(await card.respond("tentative")).toBe(false);
  expect(host.respond).not.toHaveBeenCalled();
  expect(await card.respond("decline")).toBe(true);
  expect(host.respond).toHaveBeenCalledWith("inv-1", "decline", null);
  expect(host.send.mock.calls[0]![1].to).toEqual([{ name: "Boss", email: "boss@example.com" }]);
  card.dispose();
});

test("a spoofed organizer is never answered, applied or sent anything", async () => {
  calendars();
  const spoofed = source({ from: [{ name: "Boss", email: "evil@example.net" }] });
  const request = await loaded(invitation(), spoofed);
  expect(request.check).toMatchObject({ ok: false, reason: "organizer" });
  expect(request.mayRespond).toBe(false);
  expect(await request.respond("accept")).toBe(false);
  const cancel = await loaded(invitation({ method: "cancel" }), spoofed);
  expect(cancel.mayApply).toBe(false);
  expect(await cancel.apply()).toBe(false);
  expect(host.respond).not.toHaveBeenCalled();
  expect(host.applyCancel).not.toHaveBeenCalled();
  expect(host.send).not.toHaveBeenCalled();
});

test("a reply addressed anywhere but the organizer is not sent", async () => {
  calendars();
  host.respond.mockReturnValue(ok(itip({ to: "attacker@example.net" })));
  const card = await loaded();
  expect(await card.respond("accept")).toBe(false);
  expect(host.send).not.toHaveBeenCalled();
  expect(card.send).toBe("failed");
  card.dispose();
});

test("a failed send keeps the answer for another try and a calendar refusal sends nothing", async () => {
  calendars();
  host.respond.mockReturnValue(ok(itip()));
  host.send.mockReturnValueOnce(err("offline"));
  host.send.mockReturnValueOnce(ok({ sent: true, saved_copy: true }));
  const card = await loaded();
  expect(await card.respond("accept")).toBe(false);
  expect(card.send).toBe("failed");
  expect(card.sendFailure).toBe("offline");
  expect(card.answered).toBe("accept");
  expect(await card.sendReply()).toBe(true);
  expect(card.send).toBe("sent");
  expect(host.respond).toHaveBeenCalledTimes(1);
  expect(host.send).toHaveBeenCalledTimes(2);
  expect(await card.sendReply()).toBe(false);

  host.respond.mockReset();
  host.send.mockReset();
  host.respond.mockReturnValue(err("read_only"));
  const refused = await loaded();
  expect(await refused.respond("accept")).toBe(false);
  expect(refused.calendarFailure).toBe("read_only");
  expect(host.send).not.toHaveBeenCalled();
});

test("a cancellation and an attendee's reply are applied only when asked, and say what changed", async () => {
  calendars();
  host.applyCancel.mockReturnValue(ok("cancelled"));
  const cancel = await loaded(invitation({ method: "cancel" }));
  expect(host.applyCancel).not.toHaveBeenCalled();
  expect(await cancel.apply()).toBe(true);
  expect(host.applyCancel).toHaveBeenCalledWith("inv-1");
  expect(cancel.applied).toBe("cancelled");

  host.applyReply.mockReturnValue(err("not_found"));
  const reply = await loaded(
    invitation({
      method: "reply",
      is_organizer: true,
      organizer: { email: "me@example.com", name: null, partstat: "accepted" },
      attendees: [
        {
          person: { email: "boss@example.com", name: null, partstat: "accepted" },
          role: "required",
          rsvp: false,
          is_you: false,
        },
      ],
    }),
  );
  expect(reply.mayApply).toBe(true);
  expect(await reply.apply()).toBe(false);
  expect(reply.calendarFailure).toBe("not_found");
  expect(reply.applied).toBeNull();
});

test("the reply's words are one bounded line each and carry the title", () => {
  const words = replyWords("accept", "Plan\r\nBcc: x@y.z", "Me");
  expect(words.subject).not.toMatch(/[\r\n]/u);
  expect(words.subject).toContain("Plan");
  expect(replyWords("decline", "x".repeat(1000), "Me").subject.length).toBeLessThan(260);
  expect(
    new Set([
      replyWords("accept", "T", "N").subject,
      replyWords("tentative", "T", "N").subject,
      replyWords("decline", "T", "N").subject,
    ]).size,
  ).toBe(3);
});
