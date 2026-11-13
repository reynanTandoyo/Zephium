import type { MailAddressV1 } from "$shared/ipc/bindings";

const SIMPLE_ADDRESS = /^[^\s@<>(),;:"[\]\\]+@[^\s@<>(),;:"[\]\\]+\.[^\s@<>(),;:"[\]\\]+$/u;

export const isEmailAddress = (value: string) => value.length <= 254 && SIMPLE_ADDRESS.test(value);

/** `Name <a@b.c>` or `a@b.c`, as shown and as quoted in replies. */
export function formatAddress(address: MailAddressV1): string {
  const name = address.name?.trim();
  if (!name) return address.email;
  return `${name.replace(/[<>"]/gu, "")} <${address.email}>`;
}

/** What names a person in the list: their name, else their address. */
export function displayName(address: MailAddressV1 | undefined): string {
  if (!address) return "";
  return address.name?.trim() || address.email;
}

export type ParsedAddresses = { addresses: MailAddressV1[]; invalid: string[] };

/** Reads a typed recipient field: entries split by comma or semicolon. */
export function parseAddresses(text: string): ParsedAddresses {
  const addresses: MailAddressV1[] = [];
  const invalid: string[] = [];
  const entries: string[] = [];
  let current = "";
  let quoted = false;
  let angled = false;
  for (const ch of text) {
    if (ch === '"') quoted = !quoted;
    else if (!quoted && ch === "<") angled = true;
    else if (!quoted && ch === ">") angled = false;
    if (!quoted && !angled && (ch === "," || ch === ";" || ch === "\n")) {
      entries.push(current);
      current = "";
    } else current += ch;
  }
  entries.push(current);
  for (const raw of entries) {
    const entry = raw.trim();
    if (!entry) continue;
    const named = /^(.*?)\s*<([^<>]+)>$/u.exec(entry);
    const email = (named ? named[2]! : entry).trim();
    if (!isEmailAddress(email)) {
      invalid.push(entry);
      continue;
    }
    const name = named ? named[1]!.trim().replace(/^"|"$/gu, "").trim() : "";
    addresses.push({ name: name || null, email });
  }
  return { addresses, invalid };
}

export const formatAddresses = (addresses: readonly MailAddressV1[]) =>
  addresses.map(formatAddress).join(", ");

/** Where the entry being typed starts: after the last comma, semicolon or line break. */
const entryStart = (text: string) => text.search(/[^,;\n]*$/u);

/**
 * The entry being typed in a recipient field: what follows the last comma,
 * semicolon or line break. Empty once it is a complete `Name <address>`.
 */
export function typedEntry(text: string): string {
  const entry = text.slice(entryStart(text)).trim();
  return entry.includes("<") || entry.includes(">") ? "" : entry;
}

/** The field text with the entry being typed replaced by the chosen contact. */
export function completeEntry(text: string, contact: MailAddressV1): string {
  const head = text.slice(0, entryStart(text));
  const spaced = head && !/\s$/u.test(head) ? `${head} ` : head;
  return `${spaced}${formatAddress(contact)}, `;
}
