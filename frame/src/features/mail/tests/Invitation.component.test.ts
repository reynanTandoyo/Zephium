import { afterEach, expect, test, vi } from "vitest";
import { render } from "vitest-browser-svelte";
import { page } from "vitest/browser";
import "$styles/global.css";
import type { CalendarInvitationV1 } from "$shared/ipc/bindings";
import InvitationCard from "../components/InvitationCard.svelte";
import MailPage from "../components/MailPage.svelte";
import { account, body, failed, folders, message, ok, pageOf } from "./fixtures";

const native = vi.hoisted(() => ({
  bytes: vi.fn(),
  parse: vi.fn(),
  accounts: vi.fn(),
  calendars: vi.fn(),
  respond: vi.fn(),
  applyCancel: vi.fn(),
  send: vi.fn(),
  mailAccounts: vi.fn(),
  mailFolders: vi.fn(),
  mailMessages: vi.fn(),
  mailMessage: vi.fn(),
  setFlags: vi.fn(),
}));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    mailInvitationBytes: native.bytes,
    calendarInvitationParse: native.parse,
    calendarAccounts: native.accounts,
    calendarCalendars: native.calendars,
    calendarInvitationRespond: native.respond,
    calendarInvitationApplyCancel: native.applyCancel,
    mailSend: native.send,
    mailAccounts: native.mailAccounts,
    mailFolders: native.mailFolders,
    mailMessages: native.mailMessages,
    mailMessage: native.mailMessage,
    mailSetFlags: native.setFlags,
  });
});

afterEach(() => {
  for (const mock of Object.values(native)) mock.mockReset();
});

function invitation(over: Partial<CalendarInvitationV1> = {}): CalendarInvitationV1 {
  return {
    id: "inv-1",
    method: "request",
    uid: "u1",
    sequence: 0,
    recurrence_id: null,
    summary: "Quarterly planning",
    description: "",
    location: "Room 4",
    start: "2026-11-20T09:00:00",
    end: "2026-11-20T10:00:00",
    all_day: false,
    tzid: null,
    recurring: false,
    organizer: { email: "boss@example.com", name: "Boss", partstat: "accepted" },
    attendees: [
      {
        person: { email: "me@example.com", name: "Me", partstat: "needs_action" },
        role: "required",
        rsvp: true,
        is_you: true,
      },
      {
        person: { email: "ann@example.org", name: "Ann", partstat: "accepted" },
        role: "required",
        rsvp: true,
        is_you: false,
      },
      {
        person: { email: "bob@example.org", name: "Bob", partstat: "declined" },
        role: "optional",
        rsvp: true,
        is_you: false,
      },
    ],
    own_address: "me@example.com",
    is_organizer: false,
    can_respond: true,
    existing: null,
    suggested_calendar: "cal-1",
    ...over,
  };
}

const source = (from = "boss@example.com") => ({
  account: "acc-1",
  folder: "INBOX",
  uid: 7,
  part: 1,
  from: [{ name: "Boss", email: from }],
  messageId: "m7@example.com",
  own: ["me@example.com"],
  ownName: "Me",
});

const itip = {
  to: "boss@example.com",
  subject: "Accepted: Quarterly planning",
  body_text: "",
  ics: "BEGIN:VCALENDAR\r\nMETHOD:REPLY\r\nEND:VCALENDAR\r\n",
  content_type: "text/calendar; method=REPLY; charset=UTF-8",
  calendar_action: "created" as const,
};

function setup(inv = invitation(), withCalendars = true, extraAccounts: unknown[] = []) {
  native.bytes.mockResolvedValue(ok("QkVHSU4="));
  native.parse.mockResolvedValue(ok(inv));
  native.accounts.mockResolvedValue(
    ok([
      { id: "dav", kind: "caldav", name: "Work", server_url: "", username: null },
      ...extraAccounts,
    ]),
  );
  native.calendars.mockResolvedValue(
    ok(
      withCalendars
        ? [
            {
              id: "cal-1",
              account_id: "dav",
              url: "",
              name: "Work calendar",
              color: null,
              read_only: false,
              visible: true,
            },
            {
              id: "cal-ro",
              account_id: "dav",
              url: "",
              name: "Read only",
              color: null,
              read_only: true,
              visible: true,
            },
          ]
        : [],
    ),
  );
}

test("the card shows what, when, where, who and the state of your own copy", async () => {
  setup(
    invitation({
      existing: {
        calendar_id: "cal-1",
        read_only: false,
        status: "confirmed",
        partstat: "accepted",
        freshness: "newer",
        same_organizer: true,
      },
    }),
  );
  render(InvitationCard, { source: source() });
  await expect.element(page.getByRole("heading", { name: "Quarterly planning" })).toBeVisible();
  await expect.element(page.getByText("Updated invitation")).toBeVisible();
  await expect.element(page.getByText("Room 4")).toBeVisible();
  await expect.element(page.getByText("Boss", { exact: true })).toBeVisible();
  await expect
    .element(page.getByText("Already in your calendar; this message updates it"))
    .toBeVisible();
  await page.getByText("Attendees (3)").click();
  await expect.element(page.getByText("Ann", { exact: true })).toBeVisible();
  await expect.element(page.getByText("Declined", { exact: true })).toBeVisible();
  await expect.element(page.getByText("(you)")).toBeVisible();
  expect(native.parse).toHaveBeenCalledWith("QkVHSU4=", ["me@example.com"], ["boss@example.com"]);
});

test("nothing is sent until Accept is pressed; then the calendar is answered and the reply is sent", async () => {
  setup();
  native.respond.mockResolvedValue(ok(itip));
  native.send.mockResolvedValue(ok({ sent: true, saved_copy: true }));
  render(InvitationCard, { source: source() });
  await expect.element(page.getByRole("button", { name: "Accept" })).toBeEnabled();
  expect(native.respond).not.toHaveBeenCalled();
  expect(native.send).not.toHaveBeenCalled();
  await page.getByRole("button", { name: "Accept" }).click();
  await expect
    .element(page.getByText("You accepted. The reply was sent to boss@example.com."))
    .toBeVisible();
  expect(native.respond).toHaveBeenCalledExactlyOnceWith("inv-1", "accept", "cal-1");
  const [account, draft] = native.send.mock.calls[0]!;
  expect(account).toBe("acc-1");
  expect(draft).toMatchObject({
    subject: "Accepted: Quarterly planning",
    calendar: { method: "REPLY" },
  });
  await expect.element(page.getByRole("button", { name: "Accept" })).not.toBeInTheDocument();
});

test("a spoofed organizer shows the card read-only with a warning and no buttons", async () => {
  setup();
  render(InvitationCard, { source: source("evil@example.net") });
  await expect.element(page.getByRole("alert")).toHaveTextContent(/Not verified/u);
  await expect.element(page.getByRole("alert")).toHaveTextContent(/evil@example\.net/u);
  await expect.element(page.getByRole("heading", { name: "Quarterly planning" })).toBeVisible();
  await expect.element(page.getByRole("button", { name: "Accept" })).not.toBeInTheDocument();
  await expect.element(page.getByRole("button", { name: "Decline" })).not.toBeInTheDocument();
  expect(native.respond).not.toHaveBeenCalled();
  expect(native.send).not.toHaveBeenCalled();
});

test("with no writable CalDAV calendar, accepting is off and the card says why; Google is named", async () => {
  setup(invitation(), false, [
    { id: "g", kind: "google", name: "G", server_url: "", username: null },
  ]);
  native.respond.mockResolvedValue(ok({ ...itip, subject: "Declined: Quarterly planning" }));
  native.send.mockResolvedValue(ok({ sent: true, saved_copy: false }));
  render(InvitationCard, { source: source() });
  await expect.element(page.getByText(/No calendar can store this event/u)).toBeVisible();
  await expect
    .element(page.getByText(/Google and Microsoft calendars cannot store invited events yet/u))
    .toBeVisible();
  await expect.element(page.getByRole("button", { name: "Accept" })).toBeDisabled();
  await expect.element(page.getByRole("button", { name: "Tentative" })).toBeDisabled();
  await page.getByRole("button", { name: "Decline" }).click();
  await expect.element(page.getByText(/You declined/u)).toBeVisible();
  expect(native.respond).toHaveBeenCalledWith("inv-1", "decline", null);
});

test("a reply that cannot be sent says so and can be sent again", async () => {
  setup();
  native.respond.mockResolvedValue(ok(itip));
  native.send.mockResolvedValueOnce(failed("offline"));
  native.send.mockResolvedValueOnce(ok({ sent: true, saved_copy: true }));
  render(InvitationCard, { source: source() });
  await page.getByRole("button", { name: "Accept" }).click();
  await expect
    .element(page.getByRole("alert"))
    .toHaveTextContent(/Your calendar was updated, but the reply was not sent/u);
  await page.getByRole("button", { name: "Send reply again" }).click();
  await expect.element(page.getByText(/You accepted/u)).toBeVisible();
  expect(native.respond).toHaveBeenCalledTimes(1);
  expect(native.send).toHaveBeenCalledTimes(2);
});

test("a calendar refusal is shown and nothing is sent", async () => {
  setup();
  native.respond.mockResolvedValue(failed("read_only"));
  render(InvitationCard, { source: source() });
  await page.getByRole("button", { name: "Accept" }).click();
  await expect
    .element(page.getByRole("alert"))
    .toHaveTextContent(/could not be updated \(read_only\)/u);
  expect(native.send).not.toHaveBeenCalled();
});

test("a cancellation is applied only on request, and the result is told", async () => {
  setup(invitation({ method: "cancel", can_respond: false }));
  native.applyCancel.mockResolvedValue(ok("cancelled"));
  render(InvitationCard, { source: source() });
  await expect.element(page.getByText("Meeting cancelled", { exact: true })).toBeVisible();
  await expect.element(page.getByRole("button", { name: "Accept" })).not.toBeInTheDocument();
  expect(native.applyCancel).not.toHaveBeenCalled();
  await page.getByRole("button", { name: "Apply cancellation" }).click();
  await expect.element(page.getByText("Marked as cancelled in your calendar.")).toBeVisible();
  expect(native.applyCancel).toHaveBeenCalledWith("inv-1");
});

test("a copy that was cancelled, an older invitation and an unreadable part are told plainly", async () => {
  setup(
    invitation({
      existing: {
        calendar_id: "cal-1",
        read_only: false,
        status: "cancelled",
        partstat: null,
        freshness: "same",
        same_organizer: true,
      },
    }),
  );
  const first = await render(InvitationCard, { source: source() });
  await expect.element(page.getByText("Cancelled in your calendar")).toBeVisible();
  await first.unmount();

  setup(
    invitation({
      existing: {
        calendar_id: "cal-1",
        read_only: false,
        status: "confirmed",
        partstat: null,
        freshness: "older",
        same_organizer: true,
      },
    }),
  );
  const second = await render(InvitationCard, { source: source() });
  await expect
    .element(page.getByText("Your calendar has a newer version than this message"))
    .toBeVisible();
  await expect.element(page.getByRole("button", { name: "Accept" })).not.toBeInTheDocument();
  await second.unmount();

  native.bytes.mockResolvedValue(failed("invalid_input"));
  render(InvitationCard, { source: source() });
  await expect.element(page.getByText("This calendar invitation could not be read.")).toBeVisible();
});

test("a message with an invitation part shows the card above its body", async () => {
  setup();
  const header = message(1, { from: [{ name: "Boss", email: "boss@example.com" }] });
  native.mailAccounts.mockResolvedValue(ok([account]));
  native.mailFolders.mockResolvedValue(ok(folders));
  native.mailMessages.mockResolvedValue(pageOf([header]));
  native.mailMessage.mockResolvedValue(
    ok(
      body(header, {
        invitations: [{ part: 1, method: "REQUEST", filename: "invite.ics", size: 120 }],
      }),
    ),
  );
  native.setFlags.mockResolvedValue(ok(null));
  render(MailPage);
  await page.getByRole("button", { name: /Subject 1/u }).click();
  await expect.element(page.getByRole("heading", { name: "Quarterly planning" })).toBeVisible();
  expect(native.bytes).toHaveBeenCalledWith("acc-1", "INBOX", 1, 1);
  expect(native.parse).toHaveBeenCalledWith("QkVHSU4=", ["me@example.com"], ["boss@example.com"]);
});
