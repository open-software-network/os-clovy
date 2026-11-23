import { describe, expect, it } from "vitest";
import { catalogs } from "../i18n/catalog";

// Copy the Windows app shows about CLIs, gh, memories, and activity must speak
// of the computer, not "this Mac".
const keys = [
  "lib.errors.llm.cliNotInstalled",
  "settingsPanels.connectors.viaGhTip",
  "settingsPanels.memory.blurb",
  "dayIntelligence.provider.missingDescription",
];

describe("computer-neutral copy", () => {
  for (const [locale, catalog] of Object.entries(catalogs)) {
    it(`does not say Mac in ${locale}`, () => {
      for (const key of keys) {
        const message = catalog[key];
        expect(message, key).toBeDefined();
        expect(String(message), key).not.toMatch(/\bMac\b/);
      }
    });
  }
});
