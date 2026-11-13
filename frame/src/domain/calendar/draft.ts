import type {
  CalendarEventDraftV1,
  CalendarOccurrenceV1,
  CalendarRecurrenceV1,
} from "$shared/ipc/bindings";
import {
  addDays,
  dayKey,
  dayNumber,
  parseCalendarTime,
  parseDayKey,
  startOfDay,
  timeKey,
} from "./dates";
import type { PlacedEvent } from "./dates";

const FREQUENCIES = ["none", "daily", "weekly", "monthly", "yearly"] as const;
type Frequency = (typeof FREQUENCIES)[number];
export const WEEKDAYS = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"] as const;

/** The repeat rule as the form edits it. `custom` rules (parts the structured
 *  form cannot express) are shown read-only and sent back as their RRULE. */
export type RecurrenceForm = {
  freq: Frequency | "custom";
  /** Every n-th period, as typed. */
  interval: string;
  ends: "never" | "count" | "until";
  count: string;
  /** `YYYY-MM-DD`. */
  until: string;
  /** Weekdays of a weekly rule (`MO`..`SU`); empty means the start's weekday. */
  byDay: string[];
  /** Day and month selectors the form has no controls for, kept while the
   *  frequency stays the same. */
  keep: {
    freq: string;
    by_day: string[];
    by_month_day: number[];
    by_month: number[];
  } | null;
  /** The stored RRULE of a custom rule. */
  rrule: string | null;
};

export const noRecurrence = (): RecurrenceForm => ({
  freq: "none",
  interval: "1",
  ends: "never",
  count: "10",
  until: "",
  byDay: [],
  keep: null,
  rrule: null,
});

/** Which events an edit, move or delete of a repeating event applies to. */
export type Scope = "this" | "all";

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
  recurrence: RecurrenceForm;
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
    recurrence: noRecurrence(),
    reminder: "",
    url: null,
    moreReminders: [],
    zone: null,
  };
}

function recurrenceFormOf(occurrence: CalendarOccurrenceV1): RecurrenceForm {
  const rule: CalendarRecurrenceV1 | null | undefined = occurrence.recurrence;
  if (!occurrence.recurring) return noRecurrence();
  if (!rule || rule.custom || !FREQUENCIES.includes(rule.freq as Frequency))
    return { ...noRecurrence(), freq: "custom", rrule: occurrence.rrule ?? null };
  const until = rule.until ? rule.until.slice(0, 10) : "";
  return {
    freq: rule.freq as Frequency,
    interval: String(rule.interval),
    ends: rule.count ? "count" : until ? "until" : "never",
    count: rule.count ? String(rule.count) : "10",
    until,
    byDay: rule.freq === "weekly" ? (rule.by_day ?? []) : [],
    keep: {
      freq: rule.freq,
      by_day: rule.by_day ?? [],
      by_month_day: rule.by_month_day ?? [],
      by_month: rule.by_month ?? [],
    },
    rrule: occurrence.rrule ?? null,
  };
}

/** An existing event as a form. For a repeating event, `all` edits the
 *  series (its own start, end, zone and rule); `this` edits the one
 *  occurrence, whose repeat rule is not editable. */
export function formFromEvent(event: PlacedEvent, scope: Scope = "this"): EventForm {
  const { occurrence } = event;
  const series =
    scope === "all" && occurrence.recurring && occurrence.series_start && occurrence.series_end
      ? {
          start: parseCalendarTime(occurrence.series_start),
          end: parseCalendarTime(occurrence.series_end),
        }
      : null;
  const start = series?.start ?? event.start;
  const end = series?.end ?? event.end;
  const lastDay = event.allDay ? addDays(end, -1) : end;
  return {
    calendarId: occurrence.calendar_id,
    summary: occurrence.summary,
    allDay: event.allDay,
    startDate: dayKey(start),
    startTime: timeKey(start),
    endDate: dayKey(event.allDay ? (lastDay.getTime() < start.getTime() ? start : lastDay) : end),
    endTime: timeKey(end),
    location: occurrence.location,
    description: occurrence.description,
    recurrence: scope === "all" ? recurrenceFormOf(occurrence) : noRecurrence(),
    reminder: occurrence.reminders.length > 0 ? String(occurrence.reminders[0]) : "",
    url: occurrence.url,
    moreReminders: occurrence.reminders.slice(1),
    zone: (series ? occurrence.series_tzid : occurrence.tzid) ?? occurrence.tzid,
  };
}

/** What can be done to an occurrence: change it, or only look at it. Repeating
 *  events are changed through a {@link Scope} the person picks first. */
export type EditPolicy = "edit" | "read-only";

export function editPolicy(occurrence: CalendarOccurrenceV1): EditPolicy {
  return occurrence.read_only ? "read-only" : "edit";
}

export type FormError = "range" | "reminder" | "recurrence";

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

const whole = (value: string, max: number): number | null => {
  const text = String(value).trim();
  const number = Number(text);
  return text !== "" && Number.isInteger(number) && number >= 1 && number <= max ? number : null;
};

/** The wire description of the rule, `null` for none, or null when invalid. A
 *  custom rule is sent back through `rrule` untouched. */
function recurrenceDraft(
  form: RecurrenceForm,
  startDate: string,
): { recurrence: CalendarRecurrenceV1 | null; rrule: string | null } | null {
  if (form.freq === "none") return { recurrence: null, rrule: null };
  if (form.freq === "custom") return { recurrence: null, rrule: form.rrule };
  const interval = whole(form.interval, 1000);
  if (interval === null) return null;
  const rule: CalendarRecurrenceV1 = { freq: form.freq, interval };
  if (form.ends === "count") {
    const count = whole(form.count, 1000);
    if (count === null) return null;
    rule.count = count;
  } else if (form.ends === "until") {
    const until = parseDayKey(form.until);
    const first = parseDayKey(startDate);
    if (!until || (first && until.getTime() < first.getTime())) return null;
    rule.until = dayKey(until);
  }
  const kept = form.keep && form.keep.freq === form.freq ? form.keep : null;
  if (form.freq === "weekly") rule.by_day = form.byDay;
  else if (kept) rule.by_day = kept.by_day;
  if (kept && form.freq !== "weekly") {
    rule.by_month_day = kept.by_month_day;
    rule.by_month = kept.by_month;
  }
  return { recurrence: rule, rrule: null };
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
  const repeat = recurrenceDraft(form.recurrence, form.startDate);
  if (!repeat) return { ok: false, error: "recurrence" };
  const base = {
    summary: form.summary.trim(),
    description: form.description,
    location: form.location.trim(),
    all_day: form.allDay,
    rrule: repeat.rrule,
    recurrence: repeat.recurrence,
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

/** `form` with its start and end moved by whole days and minutes. */
function shifted(form: EventForm, days: number, minutes: number): EventForm {
  const first = parseDayKey(form.startDate);
  const last = parseDayKey(form.endDate);
  if (form.allDay) {
    return {
      ...form,
      startDate: first ? dayKey(addDays(first, days)) : form.startDate,
      endDate: last ? dayKey(addDays(last, days)) : form.endDate,
    };
  }
  const start = formDate(form.startDate, form.startTime);
  const end = formDate(form.endDate, form.endTime);
  if (!start || !end) return form;
  const move = (date: Date) =>
    new Date(
      date.getFullYear(),
      date.getMonth(),
      date.getDate() + days,
      date.getHours(),
      date.getMinutes() + minutes,
    );
  const [from, to] = [move(start), move(end)];
  return {
    ...form,
    startDate: dayKey(from),
    startTime: timeKey(from),
    endDate: dayKey(to),
    endTime: timeKey(to),
  };
}

const minuteOfDay = (date: Date) => date.getHours() * 60 + date.getMinutes();

/** The form of `event` moved to start at `start`, keeping its length. With
 *  scope `all` the series moves by the same distance as the occurrence. */
export function movedForm(event: PlacedEvent, start: Date, scope: Scope = "this"): EventForm {
  const days = dayNumber(start) - dayNumber(event.start);
  const minutes = event.allDay ? 0 : minuteOfDay(start) - minuteOfDay(event.start);
  return shifted(formFromEvent(event, scope), days, minutes);
}
