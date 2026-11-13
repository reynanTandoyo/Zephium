import * as m from "$shared/i18n/messages";
import { modePlace } from "$domain/time";

/** What a listed place is called: Mail and Calendar by name, a site by its host. */
export function placeLabel(site: string): string {
  switch (modePlace(site)) {
    case "mail":
      return m.mode_mail();
    case "calendar":
      return m.mode_calendar();
    default:
      return site;
  }
}
