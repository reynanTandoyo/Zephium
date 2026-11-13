import { expect, test } from "vitest";
import type { VoiceProblem } from "$domain/voice";
import { voiceProblem } from "../lib/voice-problem";

test("a refusal of the retired hosted engine reads as dictation being unavailable", () => {
  // Stored values from before the engine was retired; typed as text because they are no longer offered.
  const stored: string[] = ["signed_out", "plan_required", "billing_review", "daily_limit"];
  for (const fault of stored) {
    const problem = voiceProblem(fault as VoiceProblem, "open_ai");
    expect(problem.message, fault).toBe("Dictation isn’t available right now.");
    expect(problem.remedy, fault).toBe("retry");
  }
});

test("a key that is out of credit points at settings", () => {
  expect(voiceProblem("out_of_credit", "open_ai").remedy).toBe("settings");
  expect(voiceProblem("out_of_credit", null).remedy).toBe("settings");
});
