import * as m from "$shared/i18n/messages";
import {
  Briefcase02Icon,
  Calendar01Icon,
  Globe02Icon,
  Mail01Icon,
} from "@hugeicons/core-free-icons";
import { surface } from "$domain/surface";
import type { ModeId } from "$domain/surface";

/** How the switch draws each mode; which modes exist and are on is the surface domain's list. */
const looks = {
  browse: { label: m.mode_browse, icon: Globe02Icon },
  work: { label: m.mode_work, icon: Briefcase02Icon },
  mail: { label: m.mode_mail, icon: Mail01Icon },
  calendar: { label: m.mode_calendar, icon: Calendar01Icon },
} satisfies Record<ModeId, { label: () => string; icon: typeof Globe02Icon }>;

export interface ModeOption {
  value: ModeId;
  label: string;
  icon: typeof Globe02Icon;
}

/** The modes whose feature is on, in switch order. Fewer than two means no switch. */
export const modeOptions = (): ModeOption[] =>
  surface.enabledModes().map((mode) => ({
    value: mode.id,
    label: looks[mode.id].label(),
    icon: looks[mode.id].icon,
  }));

/** Asks native for a mode; the switch follows once it confirms. */
export function chooseMode(id: ModeId) {
  const mode = surface.MODES.find((entry) => entry.id === id);
  if (mode) void surface.open(mode.page);
}
