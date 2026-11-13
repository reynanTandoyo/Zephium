import type { MailFolderV1 } from "$shared/ipc/bindings";
import * as m from "$shared/i18n/messages";
import type { MailFailure } from "$domain/mail";

/** What the person reads for a failed mail request. */
export function reasonText(error: MailFailure | "no_folder"): string {
  switch (error) {
    case "invalid_input":
      return m.mail_error_invalid_input();
    case "not_found":
      return m.mail_error_not_found();
    case "offline":
      return m.mail_error_offline();
    case "timeout":
      return m.mail_error_timeout();
    case "tls":
      return m.mail_error_tls();
    case "auth_failed":
      return m.mail_error_auth_failed();
    case "server_rejected":
      return m.mail_error_server_rejected();
    case "protocol":
      return m.mail_error_protocol();
    case "too_large":
      return m.mail_error_too_large();
    case "credentials":
      return m.mail_error_credentials();
    case "unsupported":
      return m.mail_error_unsupported();
    case "busy":
      return m.mail_error_busy();
    case "disabled":
      return m.mail_error_disabled();
    case "no_folder":
      return m.mail_error_no_folder();
    case "unavailable":
      return m.mail_error_unavailable();
    default:
      return m.mail_error_internal();
  }
}

/** A folder's name: its role's own word, else the name the server gave it. */
export function folderLabel(folder: Pick<MailFolderV1, "role" | "name">): string {
  switch (folder.role) {
    case "inbox":
      return m.mail_folder_inbox();
    case "drafts":
      return m.mail_folder_drafts();
    case "sent":
      return m.mail_folder_sent();
    case "archive":
      return m.mail_folder_archive();
    case "junk":
      return m.mail_folder_junk();
    case "trash":
      return m.mail_folder_trash();
    default:
      return folder.name;
  }
}

const sameDay = (a: Date, b: Date) =>
  a.getFullYear() === b.getFullYear() &&
  a.getMonth() === b.getMonth() &&
  a.getDate() === b.getDate();

/** Time today, day and month this year, full date before that. */
export function formatWhen(seconds: number | null, now = new Date()): string {
  if (seconds === null) return "";
  const date = new Date(seconds * 1000);
  if (sameDay(date, now))
    return new Intl.DateTimeFormat(undefined, { timeStyle: "short" }).format(date);
  if (date.getFullYear() === now.getFullYear()) {
    return new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" }).format(date);
  }
  return new Intl.DateTimeFormat(undefined, { dateStyle: "medium" }).format(date);
}

export const formatFull = (seconds: number | null) =>
  seconds === null
    ? ""
    : new Intl.DateTimeFormat(undefined, { dateStyle: "full", timeStyle: "short" }).format(
        new Date(seconds * 1000),
      );

export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value >= 10 ? Math.round(value) : value.toFixed(1)} ${units[unit]}`;
}

/** A tooltip that names the action and its key. */
export const tip = (label: string, keys: string) => m.mail_tip({ label, keys });
