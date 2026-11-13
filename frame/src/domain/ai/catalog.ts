import type {
  WorkKeyStateV1,
  WorkModelEntry,
  WorkModelPrice,
  WorkModelProvider,
  WorkModelRole,
  WorkModelsV1,
} from "$shared/ipc/bindings";

/** A provider the person can use: the retired hosted one is not among them. */
export type KeyedProvider = Exclude<WorkModelProvider, "cloud">;

/** The AI apps on this PC that models can come from, in the order they are shown. */
export type AppProvider = Extract<WorkModelProvider, "claude_code" | "antigravity" | "open_code">;
export const appProviders: readonly AppProvider[] = ["claude_code", "antigravity", "open_code"];

export function isApp(provider: WorkModelProvider): provider is AppProvider {
  return (appProviders as readonly WorkModelProvider[]).includes(provider);
}

/**
 * The providers Settings, the model picker and onboarding offer, in the order
 * they are shown: the AI apps, then the person's own endpoint. The built-in key
 * providers (Anthropic, OpenAI, …) keep working in Rust but are not offered.
 */
export const offeredProviders: readonly KeyedProvider[] = [...appProviders, "compatible"];

const names: Record<WorkModelProvider, string> = {
  anthropic: "Anthropic",
  open_ai: "OpenAI",
  google: "Google",
  deep_seek: "DeepSeek",
  open_router: "OpenRouter",
  xai: "xAI",
  mistral: "Mistral",
  groq: "Groq",
  cerebras: "Cerebras",
  compatible: "Custom",
  claude_code: "Claude Code",
  antigravity: "Antigravity",
  open_code: "OpenCode",
  // Deprecated: the hosted provider is gone and never produced; kept so stored data still parses.
  cloud: "Unavailable",
};

export function providerName(provider: WorkModelProvider): string {
  return names[provider];
}

/** The model's name as the trigger shows it: the mark beside it says the family. */
export function shortName(entry: WorkModelEntry): string {
  if (isApp(entry.model.provider)) return entryLabel(entry);
  const name = entry.display_name;
  for (const family of ["Claude ", "DeepSeek "]) {
    if (name.startsWith(family) && name.length > family.length) return name.slice(family.length);
  }
  return name;
}

/** A model as a list shows it: an app's models name the app, as in "Claude Code · Sonnet". */
export function entryLabel(entry: WorkModelEntry): string {
  const provider = entry.model.provider;
  if (!isApp(provider)) return entry.display_name;
  const app = providerName(provider);
  const name = entry.display_name.replace(/^Claude (?=\S)/u, "");
  return name.startsWith(app) ? name : `${app} · ${name}`;
}

export function keyState(models: WorkModelsV1 | null, provider: WorkModelProvider): WorkKeyStateV1 {
  return models?.providers.find((row) => row.provider === provider)?.key ?? "missing";
}

/** A provider can run calls now: an app is installed and signed in, a key is set and not refused. */
export function usable(models: WorkModelsV1 | null, provider: WorkModelProvider): boolean {
  if (!models) return false;
  // The hosted provider is deprecated and never usable.
  if (provider === "cloud") return false;
  if (isApp(provider))
    return models.providers.find((row) => row.provider === provider)?.app?.state === "ready";
  if (provider === "compatible")
    return (
      !!models.providers.find((row) => row.provider === "compatible")?.base &&
      keyState(models, provider) !== "invalid"
    );
  const key = keyState(models, provider);
  return key === "set" || key === "valid";
}

type ModelGroup = { provider: WorkModelProvider; entries: WorkModelEntry[] };

/** Until they are set up, these appear in the picker as needing it. */
const suggested: readonly WorkModelProvider[] = appProviders;

/**
 * The picker's list for a role: every curated model of usable providers,
 * recommended first, grouped by provider. The person's own server adds the
 * models it serves.
 * The AI apps that are not ready are listed apart, as needing set-up.
 */
export function pickerGroups(
  models: WorkModelsV1 | null,
  role: WorkModelRole,
  endpoint: readonly WorkModelEntry[] = [],
) {
  const entries = [...(models?.entries ?? []), ...endpoint];
  const ready: ModelGroup[] = [];
  const needsKey: WorkModelProvider[] = [];
  for (const provider of offeredProviders) {
    const seen = new Set<string>();
    const offered = entries
      .filter((entry) => {
        if (entry.model.provider !== provider || seen.has(entry.id)) return false;
        seen.add(entry.id);
        // Native resolves known IDs from the catalog before provider listings.
        return entry.roles.includes(role) || (role !== "decision" && entry.roles.includes("lead"));
      })
      .sort((a, b) => Number(b.recommended) - Number(a.recommended));
    if (usable(models, provider)) {
      if (offered.length) ready.push({ provider, entries: offered });
    } else if (suggested.includes(provider)) {
      needsKey.push(provider);
    }
  }
  return { ready, needsKey };
}

export function entryOf(models: WorkModelsV1 | null, id: string | null | undefined) {
  if (!models || !id) return null;
  return models.entries.find((entry) => entry.id === id) ?? null;
}

/** The efforts `id` takes as the lead, weakest first; empty when it takes none. */
export function effortLevels(models: WorkModelsV1 | null, id: string | null | undefined) {
  if (!models || !id) return [];
  return models.efforts.find((row) => row.id === id)?.levels ?? [];
}

/** A context window the way a model card says it: 1M, 400K, 128K. */
export function contextSize(tokens: number): string {
  const binary = tokens % 1024 === 0;
  if (tokens >= 1_000_000) {
    const millions = binary ? tokens / 1_048_576 : Math.floor(tokens / 100_000) / 10;
    return `${millions}M`;
  }
  return `${binary ? tokens / 1024 : Math.round(tokens / 1000)}K`;
}

/** One to four, from what a million output tokens cost; null when unpriced. */
export function priceTier(price: WorkModelPrice | null | undefined): 1 | 2 | 3 | 4 | null {
  if (!price) return null;
  const output = price.output / 1_000_000;
  if (output < 2) return 1;
  if (output < 15) return 2;
  if (output < 40) return 3;
  return 4;
}

/** Dollars per million tokens, as short as the number allows. */
export function dollars(micros: number): string {
  const value = micros / 1_000_000;
  const digits = value >= 1 ? (Number.isInteger(value) ? 0 : 2) : value >= 0.1 ? 2 : 3;
  return `$${value
    .toFixed(digits)
    .replace(/(\.\d*?)0+$/u, "$1")
    .replace(/\.$/u, "")}`;
}

/** The picker's matches for `query` across every ready provider, in the rail's order. */
export function searchEntries(groups: readonly ModelGroup[], query: string): WorkModelEntry[] {
  const words = query.toLocaleLowerCase().split(/\s+/u).filter(Boolean);
  if (!words.length) return [];
  return groups.flatMap((group) =>
    group.entries.filter((entry) => {
      const text =
        `${entry.display_name} ${providerName(group.provider)} ${entry.model.model}`.toLocaleLowerCase();
      return words.every((word) => text.includes(word));
    }),
  );
}
