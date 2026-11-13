import type { WorkModelEntry } from "$shared/ipc/bindings";

export type Tier = "light" | "balanced" | "flagship";

/**
 * A model's class from what its output costs, per million tokens. The dearest
 * of a provider's own models is never called lightweight while a cheaper
 * sibling exists: that is the provider's own flagship, whatever it costs.
 */
export function tiers(groups: readonly { entries: readonly WorkModelEntry[] }[]) {
  const out: Record<string, Tier> = {};
  for (const group of groups) {
    // A model that costs nothing per token, such as an AI app's, has no class by price.
    const priced = group.entries.filter((entry) => entry.price && entry.price.output > 0);
    const top = Math.max(...priced.map((entry) => entry.price!.output));
    for (const entry of priced) {
      const output = entry.price!.output / 1_000_000;
      let tier: Tier = output < 6 ? "light" : output < 15 ? "balanced" : "flagship";
      if (tier === "light" && priced.length > 1 && entry.price!.output === top && output >= 3)
        tier = "balanced";
      out[entry.id] = tier;
    }
  }
  return out;
}
