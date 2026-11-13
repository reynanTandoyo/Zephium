import { describe, expect, test } from "vitest";
import { DEFAULT_ESSENTIALS, SITES, SITE_URLS } from "../lib/catalog";

describe("DEFAULT_ESSENTIALS", () => {
  test("names only catalog sites, each once, each with an address", () => {
    const ids = new Set(SITES.map((site) => site.id));
    expect(new Set(DEFAULT_ESSENTIALS).size).toBe(DEFAULT_ESSENTIALS.length);
    for (const id of DEFAULT_ESSENTIALS) {
      expect(ids.has(id), id).toBe(true);
      expect(SITE_URLS[id], id).toMatch(/^https:\/\//u);
    }
  });

  test("keeps the fork's seven sites in dock order", () => {
    expect(DEFAULT_ESSENTIALS).toEqual([
      "whatsapp",
      "github",
      "youtube",
      "claude",
      "gemini",
      "spotify",
      "teams",
    ]);
  });
});
