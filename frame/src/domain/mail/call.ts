import type { MailErrorV1, OAuthErrorV1 } from "$shared/ipc/bindings";
import { observe } from "$shared/lib/observe";

/** Why a mail request did not succeed: the backend's closed set, or ours. */
export type MailFailure = MailErrorV1 | "timeout" | "unavailable";

/** Why a sign-in request did not succeed: the backend's closed set, or ours. */
export type OAuthFailure = OAuthErrorV1 | "timeout" | "unavailable";

export type MailOutcome<T> = { ok: true; value: T } | { ok: false; error: MailFailure };
export type OAuthOutcome<T> = { ok: true; value: T } | { ok: false; error: OAuthFailure };

type Reply<T, E = MailErrorV1> = { status: "ok"; data: T } | { status: "error"; error: E };

export const READ_TIMEOUT = 30_000;
export const WRITE_TIMEOUT = 60_000;
/** A server probe or a send may take a while; the outcome after this is unknown. */
export const SLOW_TIMEOUT = 90_000;
/** The browser sign-in waits up to three minutes in Rust; this outlasts it. */
export const SIGN_IN_TIMEOUT = 200_000;
/** Fetching remote images is bounded to 30 s in Rust. */
export const IMAGES_TIMEOUT = 45_000;

/** Runs one native request and settles it explicitly: success only on `ok`. */
export async function callMail<T>(
  request: () => Promise<Reply<T>>,
  milliseconds: number,
  signal?: AbortSignal,
): Promise<MailOutcome<T> | { ok: "aborted" }> {
  return callNative(request, milliseconds, signal);
}

/** The same for the sign-in commands, whose errors are their own closed set. */
export function callOAuth<T>(
  request: () => Promise<Reply<T, OAuthErrorV1>>,
  milliseconds: number,
  signal?: AbortSignal,
): Promise<OAuthOutcome<T> | { ok: "aborted" }> {
  return callNative(request, milliseconds, signal);
}

async function callNative<T, E extends string>(
  request: () => Promise<Reply<T, E>>,
  milliseconds: number,
  signal?: AbortSignal,
): Promise<
  { ok: true; value: T } | { ok: false; error: E | "timeout" | "unavailable" } | { ok: "aborted" }
> {
  const response = await observe(Promise.resolve().then(request), milliseconds, signal);
  if (response.state !== "received") {
    if (response.state === "aborted") return { ok: "aborted" };
    return { ok: false, error: response.state === "timeout" ? "timeout" : "unavailable" };
  }
  const reply = response.value;
  if (reply.status === "ok") return { ok: true, value: reply.data };
  return { ok: false, error: reply.error };
}
