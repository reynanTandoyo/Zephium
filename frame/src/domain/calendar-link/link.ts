import type { CalendarOccurrenceV1 } from "$shared/ipc/bindings";

/** Where an event can be found again: its calendar, its UID and the start of
 *  the instance (`recurrence_id`, or `start` for a single event), in the same
 *  format `calendar_events` lists them. */
export type EventLink = { calendarId: string; uid: string; at: string };

/** A task's link field only takes a web address the browser would open, so an
 *  event link is one on a reserved host (`.invalid` never resolves) that the
 *  app recognises and never navigates to. */
const HOST = "calendar.zephium.invalid";

export const linkOf = (
  occurrence: Pick<CalendarOccurrenceV1, "calendar_id" | "uid" | "recurrence_id" | "start">,
): EventLink => ({
  calendarId: occurrence.calendar_id,
  uid: occurrence.uid,
  at: occurrence.recurrence_id ?? occurrence.start,
});

/** The key the views give an occurrence; equal for an occurrence and its link. */
export const linkId = (link: EventLink): string =>
  `${link.calendarId}\u0000${link.uid}\u0000${link.at}`;

export function eventUrl(link: EventLink): string {
  const query = new URLSearchParams({ calendar: link.calendarId, uid: link.uid, at: link.at });
  return `https://${HOST}/event?${query.toString()}`;
}

/** The event a task's link address names, or null for any other address. */
export function parseEventUrl(address: string): EventLink | null {
  try {
    const url = new URL(address);
    const query = url.searchParams;
    const [calendarId, uid, at] = [query.get("calendar"), query.get("uid"), query.get("at")];
    return url.origin === `https://${HOST}` && url.pathname === "/event" && calendarId && uid && at
      ? { calendarId, uid, at }
      : null;
  } catch {
    return null;
  }
}
