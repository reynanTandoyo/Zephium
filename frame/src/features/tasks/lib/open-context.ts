import { eventOpen, parseEventUrl } from "$domain/calendar-link";

/** Opens where a task's link leads: Calendar on the event for a task made from
 *  one, otherwise the page, through `openPage`. */
export function openContext(url: string, openPage: (url: string) => void): void {
  const link = parseEventUrl(url);
  if (link) eventOpen.open(link);
  else openPage(url);
}
