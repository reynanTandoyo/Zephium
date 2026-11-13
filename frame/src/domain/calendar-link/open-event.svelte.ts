import { surface } from "$domain/surface";
import type { EventLink } from "./link";

let wanted = $state.raw<EventLink | null>(null);

/** The event another surface asked Calendar to show, until Calendar has. */
export const pending = (): EventLink | null => wanted;

/** Calendar has shown it, or the person went elsewhere. */
export const clear = () => {
  wanted = null;
};

/** Opens Calendar on an event. The request waits in this module, so it works
 *  whether Calendar is already showing or has to load first. */
export function open(link: EventLink): void {
  wanted = link;
  void surface.open("calendar");
}
