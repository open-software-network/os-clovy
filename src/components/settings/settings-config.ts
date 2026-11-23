import { type MessageKey, t } from "../../i18n";

export type SettingsTab =
  | "general"
  | "appearance"
  | "shortcuts"
  | "activity"
  | "dictation"
  | "audio"
  | "models"
  | "agent"
  | "mcp"
  | "memory"
  | "connectors"
  | "about";
const SETTINGS_TAB_LABEL_KEYS: Record<SettingsTab, MessageKey> = {
  general: "settings.tabs.general",
  appearance: "settings.appearance.title",
  shortcuts: "settings.tabs.shortcuts",
  activity: "activity.tab.label",
  dictation: "settings.tabs.dictation",
  audio: "settings.tabs.audio",
  models: "settings.tabs.models",
  agent: "settings.tabs.agent",
  mcp: "mcpServer.tab",
  memory: "settings.tabs.memory",
  connectors: "settings.tabs.connectors",
  about: "settings.tabs.about",
};

// `label` is a getter so it reads the current interface language each time a
// nav renders, instead of freezing whichever language was active at import.
function settingsTab(id: SettingsTab): { id: SettingsTab; label: string } {
  return {
    id,
    get label() {
      return t(SETTINGS_TAB_LABEL_KEYS[id]);
    },
  };
}

export const SETTINGS_TABS: { id: SettingsTab; label: string }[] = [
  settingsTab("general"),
  settingsTab("appearance"),
  settingsTab("shortcuts"),
  settingsTab("activity"),
  settingsTab("dictation"),
  settingsTab("audio"),
  settingsTab("models"),
  settingsTab("agent"),
  settingsTab("mcp"),
  settingsTab("memory"),
  settingsTab("connectors"),
  settingsTab("about"),
];

export function settingsTabsForPlatform(activitySupported: boolean) {
  return SETTINGS_TABS.filter(
    (tab) => activitySupported || (tab.id !== "activity" && tab.id !== "mcp"),
  );
}
