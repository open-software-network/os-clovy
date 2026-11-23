import { IconArrowInbox } from "central-icons/IconArrowInbox";
import { IconBroomSparkle } from "central-icons/IconBroomSparkle";
import { IconCalendarCheck } from "central-icons/IconCalendarCheck";
import { IconCalendarClock } from "central-icons/IconCalendarClock";
import { IconCloudySun } from "central-icons/IconCloudySun";
import { IconEmail1 } from "central-icons/IconEmail1";
import { IconNewspaper } from "central-icons/IconNewspaper";
import { IconSunrise } from "central-icons/IconSunrise";
import { t } from "../../i18n";
import type { ConnectorScopeBundle, ConnectorTriggerKind, RoutineTrustMode } from "../../lib/tauri";

/** The event trigger a connector template subscribes instead of (or on top
 * of) its schedule. Config keys mirror ConnectorTrigger.config. */
export type RoutineTemplateTrigger = {
  kind: ConnectorTriggerKind;
  leadMinutes?: number;
  externalOnly?: boolean;
};

/** A starter routine: opens the create editor prefilled, never creates
 * directly — prompts carry [bracketed] placeholders the user fills in, and
 * the schedule is only a sensible default. */
export type RoutineTemplate = {
  id: string;
  name: string;
  /** Card copy: what you get, one sentence. */
  description: string;
  prompt: string;
  schedule: string;
  /** Templates whose job needs machine access (files, terminal). The editor
   * preselects Unrestricted and the card carries the warm badge so the
   * access cost is visible before anything is created. */
  unrestricted?: boolean;
  /** Google feature bundles the routine needs connected before install. The
   * create page gates on these: missing grants prompt an inline connect. */
  connectorScopes?: ConnectorScopeBundle[];
  /** Default trust mode the editor preselects for a connector template. */
  trustMode?: RoutineTrustMode;
  /** Event trigger preselected in the editor's "When" picker. */
  trigger?: RoutineTemplateTrigger;
  /** "This routine can: read your mail, draft replies" — surfaced on the
   * card and in the create page so the access cost is visible up front. */
  toolSummary?: string;
  icon: typeof IconArrowInbox;
};

// Name, description, and tool summary are getters so the cards follow the
// interface language; prompts stay English (they are sent to the model).
export const ROUTINE_TEMPLATES: RoutineTemplate[] = [
  {
    id: "morning-brief",
    get name() {
      return t("routines.template.morningBrief.name");
    },
    get description() {
      return t("routines.template.morningBrief.description");
    },
    prompt:
      "Put together a short morning brief. Look through my recent sessions and notes for open loops and unanswered questions, list my open todos, and check the web for anything new that clearly matters for my work. Keep it under 200 words.",
    schedule: "0 8 * * 1-5",
    icon: IconCloudySun,
  },
  {
    id: "weekly-review",
    get name() {
      return t("routines.template.weeklyReview.name");
    },
    get description() {
      return t("routines.template.weeklyReview.description");
    },
    prompt:
      "Write my weekly review. Summarize what I worked on this week from my sessions and notes, call out decisions that got made, and list the open loops worth carrying into next week.",
    schedule: "0 16 * * 5",
    icon: IconCalendarCheck,
  },
  {
    id: "news-watch",
    get name() {
      return t("routines.template.newsWatch.name");
    },
    get description() {
      return t("routines.template.newsWatch.description");
    },
    prompt:
      "Check the web for news about [topic]. Summarize anything genuinely new since the last run, with links. If nothing meaningful happened, reply with one line saying so.",
    schedule: "0 9 * * *",
    icon: IconNewspaper,
  },
  {
    id: "memory-tidy",
    get name() {
      return t("routines.template.memoryTidy.name");
    },
    get description() {
      return t("routines.template.memoryTidy.description");
    },
    prompt:
      "Review your memories from the past week. Consolidate duplicates, flag anything stale or contradictory, and summarize what you changed.",
    schedule: "0 18 * * 0",
    icon: IconBroomSparkle,
  },
  {
    id: "morning-briefing",
    get name() {
      return t("routines.template.morningBriefing.name");
    },
    get description() {
      return t("routines.template.morningBriefing.description");
    },
    prompt:
      "Put together my morning briefing. List today's calendar events with times and who I am meeting, summarize my unread email and call out anything that needs a reply, and add one line of prep for each meeting based on my notes and recent threads. Keep it under 250 words.",
    schedule: "0 8 * * *",
    connectorScopes: ["gmail_read", "calendar_read"],
    trustMode: "read_only",
    get toolSummary() {
      return t("routines.template.tools.mailCalendar");
    },
    icon: IconSunrise,
  },
  {
    id: "auto-inbox",
    get name() {
      return t("routines.template.autoInbox.name");
    },
    get description() {
      return t("routines.template.autoInbox.description");
    },
    prompt:
      "New email just arrived. Triage the unread messages: label each by topic and urgency, archive obvious noise, and draft a short reply to anything that clearly needs one. Every label change and draft waits for my approval.",
    schedule: "0 9 * * *",
    connectorScopes: ["gmail_read", "gmail_draft", "gmail_modify"],
    trustMode: "approval",
    trigger: { kind: "email_received" },
    get toolSummary() {
      return t("routines.template.tools.inbox");
    },
    icon: IconEmail1,
  },
  {
    id: "meeting-prep",
    get name() {
      return t("routines.template.meetingPrep.name");
    },
    get description() {
      return t("routines.template.meetingPrep.description");
    },
    prompt:
      "A meeting with external guests starts soon. Look up the event, then brief me: who is attending and their company, the last email threads with them, and what my meeting notes say we discussed or promised last time. Keep it under 200 words.",
    schedule: "0 9 * * 1-5",
    connectorScopes: ["gmail_read", "calendar_read"],
    trustMode: "read_only",
    trigger: { kind: "event_upcoming", leadMinutes: 30, externalOnly: true },
    get toolSummary() {
      return t("routines.template.tools.mailCalendar");
    },
    icon: IconCalendarClock,
  },
  {
    id: "downloads-tidy",
    get name() {
      return t("routines.template.downloadsTidy.name");
    },
    get description() {
      return t("routines.template.downloadsTidy.description");
    },
    prompt:
      "Tidy my Downloads folder: sort loose files into subfolders by type (images, documents, archives, installers). List anything older than 30 days that looks like junk I could delete, but do not delete anything yourself.",
    schedule: "0 17 * * 5",
    unrestricted: true,
    icon: IconArrowInbox,
  },
];
