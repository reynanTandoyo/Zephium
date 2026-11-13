import type { CalendarOccurrenceV1 } from "$shared/ipc/bindings";

type MeetingService = "teams" | "meet" | "zoom";
export type MeetingLink = { service: MeetingService; url: string };

const URL_PATTERN = /https:\/\/[^\s<>"')\]]+/giu;

function serviceOf(url: URL): MeetingService | null {
  const host = url.hostname.toLowerCase();
  if (
    (host === "teams.microsoft.com" || host === "teams.live.com") &&
    /^\/(l\/meetup-join|meet|meetup-join)\b/u.test(url.pathname)
  )
    return "teams";
  if (host === "meet.google.com" && /^\/[a-z0-9-]{3,}/u.test(url.pathname)) return "meet";
  if (
    (host === "zoom.us" || host.endsWith(".zoom.us")) &&
    /^\/(j|my|wc\/join)\b/u.test(url.pathname)
  )
    return "zoom";
  return null;
}

/** The first Teams, Meet or Zoom address among the event's link, location and
 *  notes. Only `https` addresses count. */
export function meetingLink(
  occurrence: Pick<CalendarOccurrenceV1, "url" | "location" | "description">,
): MeetingLink | null {
  const candidates = [occurrence.url ?? "", occurrence.location, occurrence.description];
  for (const text of candidates) {
    for (const match of text.matchAll(URL_PATTERN)) {
      const raw = match[0].replace(/[.,;:!?]+$/u, "");
      try {
        const url = new URL(raw);
        const service = serviceOf(url);
        if (service) return { service, url: url.href };
      } catch {
        // Not an address after all.
      }
    }
  }
  return null;
}
