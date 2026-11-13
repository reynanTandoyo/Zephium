/** Reading calendar times as local days and events, and today's remaining
 *  events. The light half of the Calendar domain, for surfaces (Time, the new
 *  tab) that show the day but do not edit it. */
export * from "./color";
export * from "./dates";
export * from "./meeting";
export * as today from "./today.svelte";
export { openMeeting } from "./today.svelte";
