import { expect, test } from "vitest";
import type { WorkModelFaultV1 } from "$shared/ipc/bindings";
import { availabilityView, faultView } from "../lib/faults";

const model = (fault: WorkModelFaultV1, retry_after_ms: number | null = null) =>
  faultView({ reason: "model", fault, retry_after_ms });

test("a refusal of the retired hosted provider reads as unavailable and leads to Settings → AI", () => {
  for (const fault of ["hosted_usage", "signed_out", "plan_required", "daily_limit"] as const) {
    const view = model(fault);
    expect(view.message, fault).toBe("Zephium isn’t available right now.");
    expect(view.fix?.command, fault).toBe("settings.ai");
    expect(view.retry, fault).toBe("no");
  }
});

test("a rate limit waits, then offers the same question again", () => {
  expect(model("rate_limited", 12_000)).toMatchObject({ retry: "wait", fix: null, wait: 12 });
});

test("a missing or refused key leads to Settings → AI", () => {
  expect(model("missing_key").fix?.command).toBe("settings.ai");
  expect(model("key_refused").fix?.command).toBe("settings.ai");
  expect(faultView({ reason: "no_model" }).fix?.command).toBe("settings.ai");
});

test("a request over the provider plan's per-minute limit is not retried", () => {
  expect(model("plan_limit")).toMatchObject({ retry: "no", fix: { command: "settings.ai" } });
});

test("a page that changed may be asked again; no selection may not", () => {
  expect(faultView({ reason: "page_changed" }).retry).toBe("now");
  expect(faultView({ reason: "no_selection" }).retry).toBe("no");
});

test("every page that is not attached has words of its own, and says whether talk goes on", () => {
  const kinds = [
    "ai_off",
    "private",
    "sensitive",
    "not_web",
    "no_model",
    "unsupported",
    "loading",
    "unavailable",
  ] as const;
  const titles = kinds.map((kind) => availabilityView(kind).title);
  expect(new Set(titles).size).toBe(titles.length);
  expect(availabilityView("private").talks).toBe(false);
  expect(availabilityView("ai_off").talks).toBe(false);
  expect(availabilityView("sensitive").talks).toBe(true);
  expect(availabilityView("not_web").talks).toBe(true);
});

test("a reply that ran too long may be sent again", () => {
  expect(faultView({ reason: "too_long" }).retry).toBe("now");
});

test("a page still loading is asked again, not called unavailable", () => {
  const view = faultView({ reason: "loading" });
  expect(view.retry).toBe("now");
  expect(view.message).toContain("still loading");
  expect(faultView({ reason: "invalid" }).retry).toBe("no");
});
