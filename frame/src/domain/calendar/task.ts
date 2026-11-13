import * as m from "$shared/i18n/messages";
import type { TaskSession } from "$domain/resources";
import { dayKey, timeKey, type PlacedEvent } from "$domain/calendar-agenda";
import { eventUrl, linkOf } from "$domain/calendar-link";

type TaskInput = Parameters<TaskSession["create"]>[0];

/** A task for an event: its title, due when the event starts (the day only for
 *  an all-day event) and a link back, in the task's own link field, that the
 *  Tasks views recognise and open in Calendar. */
export function taskInputOf(event: PlacedEvent): TaskInput {
  const { occurrence } = event;
  const title = occurrence.summary.trim() || m.calendar_untitled();
  return {
    title,
    dueDate: dayKey(event.start),
    dueTime: event.allDay ? null : timeKey(event.start),
    context: { url: eventUrl(linkOf(occurrence)), title },
  };
}
