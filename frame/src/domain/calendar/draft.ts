import type { CalendarEventDraftV1, CalendarOccurrenceV1 } from "$shared/ipc/bindings";
import { addDays, dayKey, parseDayKey, startOfDay, timeKey } from "./dates";
import type { PlacedEvent } from "./dates";

export const RECURRENCES = ["none", "daily", "weekly", "monthly", "yearly"] as const;
export type Recurrence = (typeof RECURRENCES)[number];

const RULES: Record<Recurrence, string | null> = {
  none: null,
  daily: "FREQ=DAILY",
  weekly: "FREQ=WEEKLY",
  monthly: "FREQ=MONTHLY",
  yearly: "FREQ=YEARLY",
};

/** What the person edits. Dates are `YYYY-MM-DD` and times `HH:MM`, both local.
 *  An all-day event's end date is the last day it covers (inclusive), unlike
 *  the wire, whose end is exclusive. */
export type EventForm = {
  calendarId: string;
  summary: string;
  allDay: boolean;
  startDate: string;
  startTime: string;
  endDate: string;
  endTime: string;
  location: string;
  description: string;
  recurrence: Recurrence;
  /** Minutes before the start; empty for no reminder. */
  reminder: string;
  /** Kept as it was: the form has no field for it. */
  url: string | null;
  /** Further reminders after the first, kept as they were. */
  moreReminders: number[];
  /** The event's own IANA zone, kept so a repeating event holds its hour. */
  zone: string | null;
};

const localZone = (): string => {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
  } catch {
    return "UTC";
  }
};

/** A new event on `start`..`end`. */
export function formForSlot(calendarId: string, start: Date, end: Date, allDay = false): EventForm {
  return {
    calendarId,
    summary: "",
    allDay,
    startDate: dayKey(start),
    startTime: timeKey(start),
    endDate: dayKey(allDay ? addDays(end, -1) : end),
    endTime: timeKey(end),
    location: "",
    description: "",
    recurrence: "none",
    reminder: "",
    url: null,
    moreReminders: [],
    zone: null,
  };
}

/** An existing, non-recurring event as a form. Editing a recurring series is
 *  not offered yet: see {@link editPolicy}. */
export function formFromEvent(event: PlacedEvent): EventForm {
  const { occurrence } = event;
  const lastDay = event.allDay ? addDays(event.end, -1) : event.end;
  return {
    calendarId: occurrence.calendar_id,
    summary: occurrence.summary,
    allDay: event.allDay,
    startDate: dayKey(event.start),
    startTime: timeKey(event.start),
    endDate: dayKey(
      event.allDay
        ? lastDay.getTime() < event.start.getTime()
          ? event.start
          : lastDay
        : event.end,
    ),
    endTime: timeKey(event.end),
    location: occurrence.location,
    description: occurrence.description,
    recurrence: "none",
    reminder: occurrence.reminders.length > 0 ? String(occurrence.reminders[0]) : "",
    url: occurrence.url,
    moreReminders: occurrence.reminders.slice(1),
    zone: occurrence.tzid,
  };
}

/** What can be done to an occurrence today.
 *
 *  A series is edited as a whole through `calendar_update_event`, whose draft
 *  needs the series' own start and RRULE. Occurrences carry neither, so
 *  rewriting one would move the series to the clicked day and drop its rule.
 *  Until the backend adds occurrence-level edits (or exposes the rule),
 *  recurring events can be read and deleted but not changed. Occurrence-only
 *  editing replaces this single decision point. */
export type EditPolicy = "edit" | "series-locked" | "read-only";

export function editPolicy(occurrence: CalendarOccurrenceV1): EditPolicy {
  if (occurrence.read_only) return "read-only";
  if (occurrence.recurring) return "series-locked";
  return "edit";
}

export type FormError = "range" | "reminder";

const parseTime = (value: string): { hours: number; minutes: number } | null => {
  const match = /^(\d{2}):(\d{2})$/u.exec(value);
  if (!match) return null;
  const [hours, minutes] = [Number(match[1]), Number(match[2])];
  return hours < 24 && minutes < 60 ? { hours, minutes } : null;
};

/** Local date and time fields as an instant. */
function formDate(date: string, time: string): Date | null {
  const day = parseDayKey(date);
  const clock = parseTime(time);
  if (!day || !clock) return null;
  return new Date(day.getFullYear(), day.getMonth(), day.getDate(), clock.hours, clock.minutes);
}

/** `YYYY-MM-DDTHH:MM:SS` as the wall clock in `zone` reads at `instant`. */
export function wallClock(instant: Date, zone: string): string {
  const parts = new Intl.DateTimeFormat("en-US", {
    timeZone: zone,
    hourCycle: "h23",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).formatToParts(instant);
  const part = (type: string) => parts.find((entry) => entry.type === type)?.value ?? "00";
  return `${part("year").padStart(4, "0")}-${part("month")}-${part("day")}T${part("hour")}:${part("minute")}:${part("second")}`;
}

/** `zone` when the runtime knows it, so an odd zone name never breaks saving. */
function knownZone(zone: string | null): string | null {
  if (!zone) return null;
  try {
    new Intl.DateTimeFormat("en-US", { timeZone: zone });
    return zone;
  } catch {
    return null;
  }
}

/** The draft to send, or why the form cannot be saved. Timed events are
 *  written as wall-clock times in `zone` so a repeating event keeps its hour
 *  across daylight-saving changes. */
export function formToDraft(
  form: EventForm,
  fallbackZone = localZone(),
): { ok: true; draft: CalendarEventDraftV1 } | { ok: false; error: FormError } {
  const minutes = form.reminder.trim() === "" ? null : Number(form.reminder);
  if (minutes !== null && (!Number.isInteger(minutes) || minutes < 0 || minutes > 40_320))
    return { ok: false, error: "reminder" };
  const reminders = minutes === null ? [...form.moreReminders] : [minutes, ...form.moreReminders];
  const base = {
    summary: form.summary.trim(),
    description: form.description,
    location: form.location.trim(),
    all_day: form.allDay,
    rrule: RULES[form.recurrence],
    reminders,
    url: form.url,
  };
  if (form.allDay) {
    const first = parseDayKey(form.startDate);
    const last = parseDayKey(form.endDate);
    if (!first || !last || last.getTime() < first.getTime()) return { ok: false, error: "range" };
    return {
      ok: true,
      draft: {
        ...base,
        start: dayKey(first),
        end: dayKey(addDays(startOfDay(last), 1)),
        tzid: null,
      },
    };
  }
  const zone = knownZone(form.zone) ?? fallbackZone;
  const start = formDate(form.startDate, form.startTime);
  const end = formDate(form.endDate, form.endTime);
  if (!start || !end || end.getTime() < start.getTime()) return { ok: false, error: "range" };
  return {
    ok: true,
    draft: { ...base, start: wallClock(start, zone), end: wallClock(end, zone), tzid: zone },
  };
}

/** The form of `event` moved to start at `start`, keeping its length. */
export function movedForm(event: PlacedEvent, start: Date): EventForm {
  const form = formFromEvent(event);
  if (event.allDay) {
    const days = Math.round(
      (Date.UTC(start.getFullYear(), start.getMonth(), start.getDate()) -
        Date.UTC(event.start.getFullYear(), event.start.getMonth(), event.start.getDate())) /
        86_400_000,
    );
    const first = parseDayKey(form.startDate)!;
    const last = parseDayKey(form.endDate)!;
    return {
      ...form,
      startDate: dayKey(addDays(first, days)),
      endDate: dayKey(addDays(last, days)),
    };
  }
  const length = event.end.getTime() - event.start.getTime();
  const end = new Date(start.getTime() + length);
  return {
    ...form,
    startDate: dayKey(start),
    startTime: timeKey(start),
    endDate: dayKey(end),
    endTime: timeKey(end),
  };
}
