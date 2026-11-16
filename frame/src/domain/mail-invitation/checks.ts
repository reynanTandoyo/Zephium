import type { CalendarInvitationV1, MailAddressV1, Response } from "$shared/ipc/bindings";

const same = (a: string, b: string) => a.trim().toLowerCase() === b.trim().toLowerCase();

/**
 * Whether the message that carries an invitation may be believed. A mail
 * header is not proof, but a message whose From is not the organizer (for a
 * request or a cancellation) or not the attendee that answers (for a reply)
 * is certainly not what it claims, so it is shown and never applied.
 */
export type SenderCheck =
  | { ok: true }
  | { ok: false; reason: "no_sender" | "no_organizer" | "organizer" | "attendee"; named: string };

export function checkSender(
  invitation: Pick<CalendarInvitationV1, "method" | "organizer" | "attendees">,
  from: readonly Pick<MailAddressV1, "email">[],
): SenderCheck {
  const senders = from.map((address) => address.email);
  if (!senders.length) return { ok: false, reason: "no_sender", named: "" };
  if (invitation.method === "request" || invitation.method === "cancel") {
    const organizer = invitation.organizer?.email;
    if (!organizer) return { ok: false, reason: "no_organizer", named: "" };
    return senders.every((sender) => same(sender, organizer))
      ? { ok: true }
      : { ok: false, reason: "organizer", named: organizer };
  }
  // A reply (or a counter proposal) comes from the attendee it speaks for.
  const attendees = invitation.attendees.map((attendee) => attendee.person.email);
  if (!attendees.length) return { ok: false, reason: "attendee", named: "" };
  const mismatch = attendees.find((attendee) => !senders.every((sender) => same(sender, attendee)));
  return mismatch === undefined ? { ok: true } : { ok: false, reason: "attendee", named: mismatch };
}

/** The user can answer: a verified, current request that lists them. */
export function canRespond(invitation: CalendarInvitationV1, check: SenderCheck): boolean {
  return (
    check.ok &&
    invitation.method === "request" &&
    invitation.can_respond &&
    !invitation.is_organizer &&
    invitation.existing?.freshness !== "older"
  );
}

/** A verified cancellation or reply can be applied to the calendar. */
export function canApply(invitation: CalendarInvitationV1, check: SenderCheck): boolean {
  if (!check.ok) return false;
  if (invitation.method === "cancel") return true;
  return invitation.method === "reply" && invitation.is_organizer;
}

/** A calendar an invited event can be stored in. */
export type WritableCalendar = { id: string; name: string; color: string | null };

// eslint-disable-next-line no-control-regex
const CONTROL = /[\u0000-\u001f\u007f]+/gu;

/** Subject and text lines of a reply stay one short line each. */
export const oneLine = (text: string, max: number) =>
  text.replace(CONTROL, " ").trim().slice(0, max);

export const responseStatus = (response: Response) =>
  response === "accept" ? "accepted" : response === "decline" ? "declined" : "tentative";

const parts = (value: string) =>
  /^(\d{4})-(\d{2})-(\d{2})(?:T(\d{2}):(\d{2}):(\d{2})(Z)?)?$/u.exec(value);

/** An instant, or a floating time in the viewer's zone; null when malformed. */
export function parseWhen(value: string): { date: Date; dateOnly: boolean } | null {
  const match = parts(value);
  if (!match) return null;
  const [, year, month, day, hour, minute, second, zulu] = match;
  if (
    Number(month) < 1 ||
    Number(month) > 12 ||
    Number(day) < 1 ||
    Number(day) > 31 ||
    (hour !== undefined && (+hour > 23 || +minute! > 59 || +second! > 60))
  ) {
    return null;
  }
  if (hour === undefined) {
    return { date: new Date(Number(year), Number(month) - 1, Number(day)), dateOnly: true };
  }
  const date = zulu
    ? new Date(Date.UTC(Number(year), Number(month) - 1, Number(day), +hour, +minute!, +second!))
    : new Date(Number(year), Number(month) - 1, Number(day), +hour, +minute!, +second!);
  return Number.isNaN(date.getTime()) ? null : { date, dateOnly: false };
}

/** The event's time in the viewer's zone, as one line. */
export function timeText(
  invitation: Pick<CalendarInvitationV1, "start" | "end" | "all_day">,
  locale?: string,
): string {
  const start = parseWhen(invitation.start);
  if (!start) return "";
  const end = parseWhen(invitation.end);
  const day = new Intl.DateTimeFormat(locale, { dateStyle: "full" });
  if (invitation.all_day || start.dateOnly) {
    // The end of a whole-day event is exclusive.
    const last = end ? new Date(end.date.getTime() - 86_400_000) : start.date;
    return last.getTime() > start.date.getTime()
      ? `${day.format(start.date)} – ${day.format(last)}`
      : day.format(start.date);
  }
  const time = new Intl.DateTimeFormat(locale, { timeStyle: "short" });
  const full = new Intl.DateTimeFormat(locale, { dateStyle: "full", timeStyle: "short" });
  if (!end) return full.format(start.date);
  const sameDay = start.date.toDateString() === end.date.toDateString();
  return sameDay
    ? `${full.format(start.date)} – ${time.format(end.date)}`
    : `${full.format(start.date)} – ${full.format(end.date)}`;
}
