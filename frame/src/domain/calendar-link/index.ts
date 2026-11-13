/** Event links and the request to open one in Calendar. Kept apart from the
 *  Calendar domain so a surface that only links to an event (Tasks) does not
 *  carry the whole calendar store. */
export * from "./link";
export * as eventOpen from "./open-event.svelte";
