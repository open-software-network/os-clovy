// Every namespace catalog, merged per locale. Namespaces are split by product
// area so each file stays reviewable; keys carry their namespace as a prefix
// ("settings.theme.title") and must be unique across files (enforced by
// src/test/i18n-catalog.test.ts).

import type { Message } from "./define";
import type { InterfaceLocale } from "./locale";
import activity from "./messages/activity";
import agent from "./messages/agent";
import app from "./messages/app";
import chat from "./messages/chat";
import codingAgents from "./messages/codingAgents";
import common from "./messages/common";
import dayIntelligence from "./messages/dayIntelligence";
import hud from "./messages/hud";
import lib from "./messages/lib";
import mcpServer from "./messages/mcpServer";
import notes from "./messages/notes";
import onboarding from "./messages/onboarding";
import recorder from "./messages/recorder";
import routines from "./messages/routines";
import settings from "./messages/settings";
import settingsPanels from "./messages/settingsPanels";
import shell from "./messages/shell";

export const namespaces = {
  activity,
  agent,
  app,
  chat,
  codingAgents,
  common,
  dayIntelligence,
  hud,
  lib,
  mcpServer,
  notes,
  onboarding,
  recorder,
  routines,
  settings,
  settingsPanels,
  shell,
};

type Namespaces = typeof namespaces;
export type MessageKey = {
  [N in keyof Namespaces]: keyof Namespaces[N]["en"];
}[keyof Namespaces] &
  string;

function merge(locale: InterfaceLocale): Record<string, Message> {
  const merged: Record<string, Message> = {};
  for (const namespace of Object.values(namespaces)) {
    Object.assign(merged, namespace[locale]);
  }
  return merged;
}

export const catalogs: Record<InterfaceLocale, Record<string, Message>> = {
  en: merge("en"),
  "pt-BR": merge("pt-BR"),
};
