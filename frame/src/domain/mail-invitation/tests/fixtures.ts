import type { CalendarInvitationV1 } from "$shared/ipc/bindings";

export function invitation(over: Partial<CalendarInvitationV1> = {}): CalendarInvitationV1 {
  return {
    id: "inv-1",
    method: "request",
    uid: "u1",
    sequence: 0,
    recurrence_id: null,
    summary: "Planning",
    description: "",
    location: "Room 4",
    start: "2026-11-20T09:00:00Z",
    end: "2026-11-20T10:00:00Z",
    all_day: false,
    tzid: null,
    recurring: false,
    organizer: { email: "Boss@Example.com", name: "Boss", partstat: "accepted" },
    attendees: [
      {
        person: { email: "me@example.com", name: null, partstat: "needs_action" },
        role: "required",
        rsvp: true,
        is_you: true,
      },
    ],
    own_address: "me@example.com",
    is_organizer: false,
    can_respond: true,
    existing: null,
    suggested_calendar: "cal-1",
    ...over,
  };
}
