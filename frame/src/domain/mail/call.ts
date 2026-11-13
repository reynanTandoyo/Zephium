import type { MailErrorV1 } from "$shared/ipc/bindings";
import { observe } from "$shared/lib/observe";

/** Why a mail request did not succeed: the backend's closed set, or ours. */
export type MailFailure = MailErrorV1 | "timeout" | "unavailable";

export type MailOutcome<T> = { ok: true; value: T } | { ok: false; error: MailFailure };

type Reply<T> = { status: "ok"; data: T } | { status: "error"; error: MailErrorV1 };

export const READ_TIMEOUT = 30_000;
export const WRITE_TIMEOUT = 60_000;
/** A server probe or a send may take a while; the outcome after this is unknown. */
export const SLOW_TIMEOUT = 90_000;

/** Runs one native request and settles it explicitly: success only on `ok`. */
export async function callMail<T>(
  request: () => Promise<Reply<T>>,
  milliseconds: number,
  signal?: AbortSignal,
): Promise<MailOutcome<T> | { ok: "aborted" }> {
  const response = await observe(Promise.resolve().then(request), milliseconds, signal);
  if (response.state === "aborted") return { ok: "aborted" };
  if (response.state === "timeout") return { ok: false, error: "timeout" };
  if (response.state === "failed") return { ok: false, error: "unavailable" };
  const reply = response.value;
  if (reply.status === "ok") return { ok: true, value: reply.data };
  return { ok: false, error: reply.error };
}
