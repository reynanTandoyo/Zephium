import { describe, expect, test } from "vitest";
import type { WorkModelEntry, WorkModelsV1 } from "$shared/ipc/bindings";
import {
  appProviders,
  contextSize,
  dollars,
  effortLevels,
  entryLabel,
  offeredProviders,
  pickerGroups,
  priceTier,
  providerName,
  searchEntries,
  shortName,
  usable,
} from "../catalog";
import { entry, models } from "$shared/testing/work-models";

describe("the picker's list", () => {
  test("a provider listing cannot promote a catalog model into another role", () => {
    const known = entry("claude_code", "haiku", "Claude Haiku", true, ["page", "light"]);
    const view = { ...models({ apps: { claude_code: "ready" } }), entries: [known] };
    const listed: WorkModelEntry[] = [{ ...known, roles: ["lead", "page", "light"] }];
    expect(pickerGroups(view, "lead", listed).ready).toEqual([]);
    expect(pickerGroups(view, "page", listed).ready[0]!.entries).toEqual([known]);
    expect(pickerGroups(view, "light", listed).ready[0]!.entries).toEqual([known]);
  });

  test("offers only the AI apps and the person's own endpoint", () => {
    expect(offeredProviders).toEqual(["claude_code", "antigravity", "open_code", "compatible"]);
    expect(appProviders).toEqual(["claude_code", "antigravity", "open_code"]);
    // The built-in key providers keep working underneath but are never listed, key or not.
    const view = models({ keys: { anthropic: "valid", open_ai: "set" } });
    const { ready, needsKey } = pickerGroups(view, "lead");
    expect(ready).toEqual([]);
    expect(needsKey).toEqual(["claude_code", "antigravity", "open_code"]);
  });

  test("groups the models of ready apps, recommended first", () => {
    const view = models({ apps: { claude_code: "ready", antigravity: "sign_in_needed" } });
    const { ready, needsKey } = pickerGroups(view, "lead");
    expect(ready.map((group) => group.provider)).toEqual(["claude_code"]);
    expect(ready[0]!.entries.map((item) => item.id)).toEqual([
      "claude-code/sonnet",
      "claude-code/opus",
    ]);
    // An app that still needs a step reads as needing set-up, not as broken.
    expect(needsKey).toEqual(["antigravity", "open_code"]);
  });

  test("an app is usable only while it is installed and signed in", () => {
    const states = ["missing", "sign_in_needed", "ready", "unknown"] as const;
    for (const state of states) {
      const view = models({ apps: { claude_code: state } });
      expect(usable(view, "claude_code")).toBe(state === "ready");
    }
    const view = models({ apps: { antigravity: "ready" } });
    expect(usable(view, "antigravity")).toBe(true);
    expect(usable(view, "claude_code")).toBe(false);
    expect(usable(view, "open_code")).toBe(false);
    expect(usable(null, "claude_code")).toBe(false);
    // A key does not make an app usable, and an app does not make a key provider usable.
    expect(
      usable(
        { ...view, providers: view.providers.map((row) => ({ ...row, key: "set" })) },
        "open_code",
      ),
    ).toBe(false);
    expect(usable(view, "anthropic")).toBe(false);
  });

  test("the retired hosted provider is never offered or usable", () => {
    const hosted = entry("cloud", "claude-sonnet-5-5", "Claude Sonnet 5.5", true);
    const view = models({ apps: { claude_code: "ready" } });
    const stored = { ...view, entries: [hosted, ...view.entries] };
    const { ready, needsKey } = pickerGroups(stored, "lead");
    expect(ready.map((group) => group.provider)).toEqual(["claude_code"]);
    expect(needsKey).not.toContain("cloud");
    expect(usable(stored, "cloud")).toBe(false);
  });

  test("a custom server adds the models it serves", () => {
    const view = models({ base: "http://localhost:11434/v1" });
    const served = [entry("compatible", "qwen4:32b", "qwen4:32b", false)];
    const { ready } = pickerGroups(view, "lead", served);
    expect(ready.map((group) => group.provider)).toEqual(["compatible"]);
    expect(ready[0]!.entries[0]!.display_name).toBe("qwen4:32b");
  });

  test("the apps come before the custom server", () => {
    const view = models({ base: "http://localhost:11434/v1", apps: { antigravity: "ready" } });
    const served = [entry("compatible", "qwen4:32b", "qwen4:32b", false)];
    expect(pickerGroups(view, "lead", served).ready.map((group) => group.provider)).toEqual([
      "antigravity",
      "compatible",
    ]);
  });

  test("a custom server is usable once it has an address, keys only when set or valid", () => {
    const view: WorkModelsV1 = models({
      keys: { deep_seek: "invalid" },
      base: "http://localhost:11434/v1",
    });
    expect(usable(view, "compatible")).toBe(true);
    expect(usable(view, "deep_seek")).toBe(false);
    expect(usable(null, "anthropic")).toBe(false);
  });

  test("an app's models name the app, and the trigger drops a family name the mark already says", () => {
    const sonnet = entry("claude_code", "sonnet", "Claude Sonnet", true);
    expect(entryLabel(sonnet)).toBe("Claude Code · Sonnet");
    expect(shortName(sonnet)).toBe("Claude Code · Sonnet");
    expect(entryLabel(entry("antigravity", "gemini-3-pro", "Gemini 3 Pro", true))).toBe(
      "Antigravity · Gemini 3 Pro",
    );
    expect(entryLabel(entry("open_code", "x", "OpenCode Zen", true))).toBe("OpenCode Zen");
    expect(providerName("open_code")).toBe("OpenCode");
    const opus: WorkModelEntry = entry("anthropic", "claude-opus-5-5", "Claude Opus 5.5", true);
    expect(shortName(opus)).toBe("Opus 5.5");
    expect(entryLabel(opus)).toBe("Claude Opus 5.5");
    expect(shortName(entry("open_ai", "gpt-6-sol", "GPT-6 Sol", true))).toBe("GPT-6 Sol");
  });
});

describe("the picker's details", () => {
  test("context windows and prices read the way a model card says them", () => {
    expect(contextSize(1_000_000)).toBe("1M");
    expect(contextSize(1_048_576)).toBe("1M");
    expect(contextSize(1_050_000)).toBe("1M");
    expect(contextSize(200_000)).toBe("200K");
    expect(contextSize(131_072)).toBe("128K");
    expect(dollars(2_000_000)).toBe("$2");
    expect(dollars(1_320_000)).toBe("$1.32");
    expect(dollars(100_000)).toBe("$0.1");
    expect(dollars(10_000)).toBe("$0.01");
    expect(priceTier({ input: 100_000, cached_input: 10_000, output: 500_000 })).toBe(1);
    expect(priceTier({ input: 2_000_000, cached_input: 200_000, output: 10_000_000 })).toBe(2);
    expect(priceTier({ input: 4_000_000, cached_input: 200_000, output: 20_000_000 })).toBe(3);
    expect(priceTier({ input: 10_000_000, cached_input: 1_000_000, output: 50_000_000 })).toBe(4);
    expect(priceTier(null)).toBeNull();
  });

  test("search finds models across providers by name, maker or id", () => {
    const view = models({ apps: { claude_code: "ready", antigravity: "ready" } });
    const { ready } = pickerGroups(view, "lead");
    expect(searchEntries(ready, "sonnet").map((item) => item.id)).toEqual(["claude-code/sonnet"]);
    expect(searchEntries(ready, "antigravity pro").map((item) => item.id)).toEqual([
      "antigravity/gemini-3-pro",
    ]);
    expect(searchEntries(ready, "  ")).toEqual([]);
  });

  test("each entry offers the efforts Rust says it takes", () => {
    const view = models({ keys: { deep_seek: "valid" }, lead: "deepseek/deepseek-v4-pro" });
    expect(effortLevels(view, "deepseek/deepseek-v4-pro")).toEqual(["low", "high"]);
    expect(effortLevels(view, "nobody/none")).toEqual([]);
    expect(view.effort).toBe("high");
  });
});
