import type { PlacedEvent } from "$domain/calendar";

/**
 * Gives every placed event a distinct id. Two resources of one calendar can
 * carry the same UID and start (poor ICS feeds do), which gives them the same
 * id (calendar, event and instance); as the key of a list row, the handle of
 * a drag or the mark of a selection it must name one row. The first keeps its
 * id and the next ones get `~2`, `~3`, in the order they arrive, which is the
 * server's order and so stable between renders.
 */
export function withUniqueIds(events: readonly PlacedEvent[]): PlacedEvent[] {
  const seen = new Map<string, number>();
  return events.map((event) => {
    const count = (seen.get(event.id) ?? 0) + 1;
    seen.set(event.id, count);
    return count === 1 ? event : { ...event, id: `${event.id}~${count}` };
  });
}
