import { expect, test } from "vitest";
import { handleNativeSection, section } from "../lib/settings-state.svelte";

test("every settings page a menu or command names is the one that opens", () => {
  for (const id of [
    "general",
    "about",
    "shortcuts",
    "profiles",
    "newtab",
    "ai",
    "sites",
    "focus",
    "appearance",
    "tabs",
    "performance",
    "work",
    "skills",
    "memory",
    "mcp",
    "features",
    "search",
    "privacy",
  ]) {
    handleNativeSection(`settings.section.${id}`);
    expect(section()).toBe(id);
  }
  handleNativeSection("settings.section.unknown");
  expect(section()).toBe("privacy");
  // The Zephium account page is gone, so its old id opens nothing.
  handleNativeSection("settings.section.account");
  expect(section()).toBe("privacy");
});

test("Search opens at Search whichever page Settings was last left on", () => {
  handleNativeSection("settings.section.appearance");
  handleNativeSection("settings.section.search");
  expect(section()).toBe("search");
});
