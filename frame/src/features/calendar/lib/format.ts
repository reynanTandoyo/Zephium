import { getLocale } from "$shared/i18n/runtime";
import * as m from "$shared/i18n/messages";
import type { CalendarFault, CalendarView, PlacedEvent } from "$domain/calendar";
import { addDays, dayNumber } from "$domain/calendar";

const formatter = (options: Intl.DateTimeFormatOptions) =>
  new Intl.DateTimeFormat(getLocale(), options);

/** `Sat`, `Sam.`, `土`: the weekday in the interface language. */
export const weekdayShort = (date: Date) => formatter({ weekday: "short" }).format(date);
export const dayOfMonth = (date: Date) => formatter({ day: "numeric" }).format(date);
export const monthTitle = (date: Date) =>
  formatter({ month: "long", year: "numeric" }).format(date);
export const dayTitle = (date: Date) =>
  formatter({ weekday: "long", day: "numeric", month: "long", year: "numeric" }).format(date);
export const dayHeading = (date: Date) =>
  formatter({ weekday: "long", day: "numeric", month: "long" }).format(date);
export const timeLabel = (date: Date) =>
  formatter({ hour: "numeric", minute: "2-digit" }).format(date);

/** The hour label of a grid row, such as `9 AM` or `09:00`. */
export const hourLabel = (hour: number) =>
  formatter({ hour: "numeric" }).format(new Date(2026, 0, 1, hour));

/** The title above a view: the month, the day, or the span of days. */
export function viewTitle(view: CalendarView, days: readonly Date[], anchor: Date): string {
  if (view === "month") return monthTitle(anchor);
  if (view === "day") return dayTitle(days[0]!);
  const first = days[0]!;
  const last = days[days.length - 1]!;
  return formatter({ day: "numeric", month: "short", year: "numeric" }).formatRange(first, last);
}

/** The time line of an event: all-day, a span of dates, or from–to. */
export function eventWhen(event: PlacedEvent): string {
  if (event.allDay) {
    const last = addDays(event.end, -1);
    return dayNumber(last) > dayNumber(event.start)
      ? formatter({ day: "numeric", month: "short", year: "numeric" }).formatRange(
          event.start,
          last,
        )
      : `${dayTitle(event.start)} · ${m.calendar_all_day()}`;
  }
  const sameDay = dayNumber(event.start) === dayNumber(event.end);
  if (sameDay)
    return `${dayTitle(event.start)} · ${formatter({ hour: "numeric", minute: "2-digit" }).formatRange(event.start, event.end)}`;
  return formatter({
    weekday: "short",
    day: "numeric",
    month: "short",
    hour: "numeric",
    minute: "2-digit",
  }).formatRange(event.start, event.end);
}

/** Reminder offsets in the words the editor offers. */
export function reminderLabel(minutes: number): string {
  if (minutes === 0) return m.calendar_reminder_at_start();
  if (minutes % 1440 === 0) return m.calendar_reminder_days({ count: minutes / 1440 });
  if (minutes % 60 === 0) return m.calendar_reminder_hours({ count: minutes / 60 });
  return m.calendar_reminder_minutes({ count: minutes });
}

/** What a failed operation says to the person. */
export function faultText(fault: CalendarFault): string {
  switch (fault) {
    case "network":
    case "timeout":
    case "server":
    case "not_found":
      return m.calendar_fault_network();
    case "unauthorized":
    case "forbidden":
    case "credentials":
      return m.calendar_fault_auth();
    case "conflict":
      return m.calendar_fault_conflict();
    case "insecure":
      return m.calendar_fault_insecure();
    case "invalid_input":
    case "too_large":
    case "limit":
    case "parse":
      return m.calendar_fault_invalid();
    case "read_only":
      return m.calendar_read_only_note();
    case "storage":
      return m.calendar_fault_storage();
    case "unsupported":
    case "unavailable":
      return m.calendar_fault_unavailable();
  }
}

/** A calendar colour from the server, or the accent when it is not `#RRGGBB`. */
export const safeColor = (color: string | null): string =>
  color !== null && /^#[0-9a-f]{6}$/iu.test(color) ? color : "var(--color-accent)";
