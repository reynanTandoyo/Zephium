import { preferences } from "$domain/preferences";
import type { BrowserPage } from "./surface.svelte";

type Feature = preferences.Feature;

/** A page that takes the whole window as a mode, beside Browse. */
export type ModePage = Extract<BrowserPage, "work" | "mail" | "calendar">;
export type ModeId = "browse" | ModePage;
export interface ModeEntry {
  id: ModeId;
  /** The browser page the mode shows; Browse is no page at all. */
  page: ModePage | null;
  /** The feature switch the mode leaves with. Browse has none. */
  feature: Feature | null;
}

/** The modes in the order the switch lists them. Mirrors `BrowserPage::is_mode` in Rust. */
export const MODES: readonly ModeEntry[] = [
  { id: "browse", page: null, feature: null },
  { id: "work", page: "work", feature: "work" },
  { id: "mail", page: "mail", feature: "mail" },
  { id: "calendar", page: "calendar", feature: "calendar" },
];

export const isModePage = (page: BrowserPage | null): page is ModePage =>
  page === "work" || page === "mail" || page === "calendar";
/** The mode a page belongs to; any other page is a destination inside Browse. */
export const modeOf = (page: BrowserPage | null): ModeId => (isModePage(page) ? page : "browse");
/** The modes whose feature is on; Browse is always among them. */
export const enabledModes = (
  isOn: (feature: Feature) => boolean = preferences.enabled,
): ModeEntry[] => MODES.filter((mode) => mode.feature === null || isOn(mode.feature));
