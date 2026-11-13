import type {
  CalendarAccountV1,
  CalendarDiscoveryV1,
  CalendarError,
  CalendarEventDraftV1,
  CalendarOccurrenceV1,
  CalendarV1,
} from "$shared/ipc/bindings";
import { commands } from "$shared/ipc/bindings";
import { events } from "$shared/ipc/native-events";

/** A failed operation: the backend's reason, or `unavailable` when the call
 *  itself could not be made. */
export type CalendarFault = CalendarError | "unavailable";
export type CalendarResult<T = null> = { ok: true; value: T } | { ok: false; error: CalendarFault };

export type LoadState = "idle" | "loading" | "ready" | "failed";
export type Busy = "sync" | "account" | "save" | "delete" | "visibility" | null;

let accountList = $state.raw<CalendarAccountV1[]>([]);
let calendarList = $state.raw<CalendarV1[]>([]);
let occurrenceList = $state.raw<CalendarOccurrenceV1[]>([]);
let cut = $state(false);
let listsState = $state<LoadState>("idle");
let eventsState = $state<LoadState>("idle");
let listsFault = $state<CalendarFault | null>(null);
let eventsFault = $state<CalendarFault | null>(null);
let working = $state<Busy>(null);
let syncFaults = $state.raw<string[]>([]);

let range: { from: string; to: string } | null = null;
let listsToken = 0;
let eventsToken = 0;
let lifecycle = 0;
let initialized = false;
let initializing: Promise<void> | null = null;
let unlisten: (() => void) | null = null;

/** Accounts and subscriptions, without secrets. */
export const accounts = (): readonly CalendarAccountV1[] => accountList;
/** Every calendar with its colour, read-only and visibility flags. */
export const calendars = (): readonly CalendarV1[] => calendarList;
/** Calendars that accept new events. */
export const writableCalendars = (): CalendarV1[] =>
  calendarList.filter((entry) => !entry.read_only);
/** Occurrences in the loaded range, from visible calendars. */
export const occurrences = (): readonly CalendarOccurrenceV1[] => occurrenceList;
/** A size limit cut the loaded occurrences short. */
export const truncated = () => cut;
/** Accounts and calendars. */
export const listState = (): LoadState => listsState;
export const listFault = (): CalendarFault | null => listsFault;
/** Events of the loaded range. */
export const eventState = (): LoadState => eventsState;
export const eventFault = (): CalendarFault | null => eventsFault;
/** The operation in flight, or null. One at a time: no queueing, no guessing. */
export const busy = (): Busy => working;
/** Calendars the last sync could not refresh. */
export const syncFailures = (): readonly string[] => syncFaults;

const fault = (error: unknown): CalendarFault =>
  typeof error === "string" ? (error as CalendarError) : "unavailable";

async function call<T>(
  run: () => Promise<{ status: "ok"; data: T } | { status: "error"; error: CalendarError }>,
): Promise<CalendarResult<T>> {
  try {
    const result = await run();
    return result.status === "ok"
      ? { ok: true, value: result.data }
      : { ok: false, error: fault(result.error) };
  } catch {
    return { ok: false, error: "unavailable" };
  }
}

async function loadLists(): Promise<void> {
  const token = ++listsToken;
  if (listsState !== "ready") listsState = "loading";
  const [accountResult, calendarResult] = await Promise.all([
    call(() => commands.calendarAccounts()),
    call(() => commands.calendarCalendars()),
  ]);
  // A newer read, or a disposal, outranks this one.
  if (token !== listsToken) return;
  if (accountResult.ok && calendarResult.ok) {
    accountList = accountResult.value;
    calendarList = calendarResult.value;
    listsState = "ready";
    listsFault = null;
  } else {
    listsState = "failed";
    listsFault = (!accountResult.ok ? accountResult : (calendarResult as { error: CalendarFault }))
      .error;
  }
}

async function loadEvents(): Promise<void> {
  if (range === null) return;
  const token = ++eventsToken;
  const { from, to } = range;
  if (eventsState !== "ready") eventsState = "loading";
  const result = await call(() => commands.calendarEvents(from, to));
  if (token !== eventsToken) return;
  if (result.ok) {
    occurrenceList = result.value.occurrences;
    cut = result.value.truncated;
    eventsState = "ready";
    eventsFault = null;
  } else {
    eventsState = "failed";
    eventsFault = result.error;
  }
}

/** Reads accounts, calendars and the current range again. */
export const refresh = (): Promise<void> => Promise.all([loadLists(), loadEvents()]).then(() => {});

/** Sets the range of instants to show (`[from, to)` as RFC 3339) and loads it.
 *  The previous occurrences stay up while the new ones load. */
export function setRange(from: string, to: string): Promise<void> {
  if (range?.from === from && range.to === to && eventsState !== "failed") return Promise.resolve();
  range = { from, to };
  return loadEvents();
}

/** Retries the range that failed. */
export const retry = (): Promise<void> => refresh();

async function run<T>(kind: Exclude<Busy, null>, action: () => Promise<CalendarResult<T>>) {
  if (working !== null) return { ok: false, error: "unavailable" } as CalendarResult<T>;
  working = kind;
  try {
    const result = await action();
    // Nothing is shown as saved until the backend says so; then read it back.
    if (result.ok) await refresh();
    return result;
  } finally {
    working = null;
  }
}

/** Finds the login address for an email address or URL. */
export const discover = (emailOrUrl: string): Promise<CalendarResult<CalendarDiscoveryV1>> =>
  call(() => commands.calendarDiscover(emailOrUrl.trim()));

/** Signs in to a CalDAV server. The password is passed through and kept nowhere. */
export const addAccount = (
  serverUrl: string,
  username: string,
  password: string,
  name: string | null,
) =>
  run("account", () =>
    call(() => commands.calendarAddAccount(serverUrl.trim(), username.trim(), password, name)),
  );

export const addSubscription = (url: string, name: string) =>
  run("account", () => call(() => commands.calendarAddSubscription(url.trim(), name.trim())));

export const removeAccount = (account: string) =>
  run("account", () => call(() => commands.calendarRemoveAccount(account)));

export const setVisible = (calendar: string, visible: boolean) =>
  run("visibility", () => call(() => commands.calendarSetVisible(calendar, visible)));

/** Refreshes every calendar from its server. */
export const sync = () =>
  run("sync", async () => {
    const result = await call(() => commands.calendarSync());
    syncFaults = result.ok
      ? result.value.outcomes
          .filter((entry) => entry.error !== null)
          .map((entry) => entry.calendar_id)
      : [];
    return result;
  });

export const createEvent = (calendar: string, draft: CalendarEventDraftV1) =>
  run("save", () => call(() => commands.calendarCreateEvent(calendar, draft)));

export const updateEvent = (calendar: string, uid: string, draft: CalendarEventDraftV1) =>
  run("save", () => call(() => commands.calendarUpdateEvent(calendar, uid, draft)));

/** Deletes the event; a recurring event is deleted as a whole series. */
export const deleteEvent = (calendar: string, uid: string) =>
  run("delete", () => call(() => commands.calendarDeleteEvent(calendar, uid)));

/** Opens a meeting address in a new browser tab. */
export const openLink = (url: string): Promise<boolean> =>
  commands.browserOpenUrl(url, true).then(
    (admission) => admission.accepted,
    () => false,
  );

async function initialize(generation: number) {
  // Listen before the first read: a change landing mid-read must not be lost.
  const stop = await events.calendarChanged.listen(() => {
    if (generation === lifecycle) void refresh();
  });
  if (generation !== lifecycle) {
    stop();
    return;
  }
  unlisten = stop;
  initialized = true;
  await loadLists();
}

export function init(): Promise<void> {
  if (initializing !== null) return initializing;
  if (initialized) return Promise.resolve();
  const generation = ++lifecycle;
  const task = initialize(generation).finally(() => {
    if (initializing === task) initializing = null;
  });
  initializing = task;
  return task;
}

export function dispose() {
  lifecycle += 1;
  listsToken += 1;
  eventsToken += 1;
  initialized = false;
  initializing = null;
  unlisten?.();
  unlisten = null;
  range = null;
  accountList = [];
  calendarList = [];
  occurrenceList = [];
  cut = false;
  listsState = "idle";
  eventsState = "idle";
  listsFault = null;
  eventsFault = null;
  working = null;
  syncFaults = [];
}
