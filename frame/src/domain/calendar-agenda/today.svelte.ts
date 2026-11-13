import type { CalendarOccurrenceV1 } from "$shared/ipc/bindings";
import { commands } from "$shared/ipc/bindings";
import { events } from "$shared/ipc/native-events";
import { addDays, placeAll, startOfDay, type PlacedEvent } from "./dates";
import { meetingLink, type MeetingLink } from "./meeting";

/** Minutes before the start from which a meeting offers Join. */
const JOIN_WINDOW_MINUTES = 10;

const TICK_MS = 30_000;

let accountCount = $state<number | null>(null);
let occurrences = $state.raw<CalendarOccurrenceV1[]>([]);
let clock = $state(Date.now());
let day = startOfDay(new Date());

let users = 0;
let lifecycle = 0;
let timer: ReturnType<typeof setInterval> | null = null;
let unlisten: (() => void) | null = null;
let token = 0;

/** Some calendar account exists, so there is a day to show. */
export const available = (): boolean => accountCount !== null && accountCount > 0;

/** The events of today that have not ended, all-day ones first, then by start.
 *  Cancelled events and hidden calendars are not in the list. */
export function upcoming(): PlacedEvent[] {
  return placeAll(occurrences)
    .filter((event) => event.occurrence.status !== "cancelled")
    .filter((event) => event.end.getTime() > clock)
    .sort(
      (a, b) =>
        Number(b.allDay) - Number(a.allDay) ||
        a.start.getTime() - b.start.getTime() ||
        a.id.localeCompare(b.id),
    );
}

/** The meeting address of an event that starts within ten minutes (or is on
 *  now), or null. */
export function joinable(event: PlacedEvent): MeetingLink | null {
  if (event.allDay) return null;
  const until = event.start.getTime() - clock;
  if (until > JOIN_WINDOW_MINUTES * 60_000 || event.end.getTime() <= clock) return null;
  return meetingLink(event.occurrence);
}

const attempt = <T>(run: () => Promise<{ status: string; data?: T }>): Promise<T | null> => {
  try {
    return run().then(
      (result) => (result.status === "ok" ? (result.data as T) : null),
      () => null,
    );
  } catch {
    // A command that cannot even be called is a failed read like any other.
    return Promise.resolve(null);
  }
};

async function load(): Promise<void> {
  const mine = ++token;
  day = startOfDay(new Date());
  const [accounts, listed] = await Promise.all([
    attempt(() => commands.calendarAccounts()),
    attempt(() => commands.calendarEvents(day.toISOString(), addDays(day, 1).toISOString())),
  ]);
  if (mine !== token) return;
  // A failed read leaves what was there; a surface this small has no error state.
  if (accounts !== null) accountCount = accounts.length;
  if (listed !== null) occurrences = listed.occurrences;
}

function start() {
  const generation = ++lifecycle;
  clock = Date.now();
  void events.calendarChanged
    .listen(() => {
      if (generation === lifecycle) void load();
    })
    .then((stop) => {
      if (generation === lifecycle) unlisten = stop;
      else stop();
    });
  timer = setInterval(() => {
    clock = Date.now();
    // Past midnight, "today" is another day.
    if (startOfDay(new Date()).getTime() !== day.getTime()) void load();
  }, TICK_MS);
  void load();
}

function stop() {
  lifecycle += 1;
  token += 1;
  if (timer) clearInterval(timer);
  timer = null;
  unlisten?.();
  unlisten = null;
  accountCount = null;
  occurrences = [];
}

/** Opens a meeting address in a new browser tab; false when it was refused. */
export const openMeeting = (url: string): Promise<boolean> =>
  commands.browserOpenUrl(url, true).then(
    (admission) => admission.accepted,
    () => false,
  );

/** Starts reading today's events for a surface that shows them; call the
 *  returned function when it goes. Surfaces share one read. */
export function acquire(): () => void {
  users += 1;
  if (users === 1) start();
  let released = false;
  return () => {
    if (released) return;
    released = true;
    users -= 1;
    if (users === 0) stop();
  };
}
