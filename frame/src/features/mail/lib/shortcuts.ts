export type MailShortcut =
  | "next"
  | "previous"
  | "compose"
  | "reply"
  | "reply_all"
  | "forward"
  | "toggle_read"
  | "toggle_flag"
  | "archive"
  | "delete"
  | "junk"
  | "search"
  | "sync";

/** The keys, as the tooltips show them. */
export const KEYS: Readonly<Record<MailShortcut, string>> = {
  next: "J",
  previous: "K",
  compose: "C",
  reply: "R",
  reply_all: "Shift+R",
  forward: "F",
  toggle_read: "U",
  toggle_flag: "S",
  archive: "E",
  delete: "#",
  junk: "!",
  search: "/",
  sync: "G",
};

const BY_KEY: Readonly<Record<string, MailShortcut>> = {
  j: "next",
  k: "previous",
  c: "compose",
  r: "reply",
  R: "reply_all",
  f: "forward",
  u: "toggle_read",
  s: "toggle_flag",
  e: "archive",
  "#": "delete",
  Delete: "delete",
  "!": "junk",
  "/": "search",
  g: "sync",
  ArrowDown: "next",
  ArrowUp: "previous",
};

/** Typing in a field is never a shortcut. */
const typing = (target: EventTarget | null) =>
  target instanceof HTMLElement &&
  (target.isContentEditable || /^(?:input|textarea|select)$/iu.test(target.tagName));

/** The action a key press means in the message list, or null. */
export function shortcutFor(event: KeyboardEvent): MailShortcut | null {
  if (event.defaultPrevented || event.ctrlKey || event.metaKey || event.altKey) return null;
  if (typing(event.target)) return null;
  // Buttons and menu items keep Enter and Space; arrows and letters stay ours.
  return BY_KEY[event.key] ?? null;
}
