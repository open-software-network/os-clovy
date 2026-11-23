/**
 * The three legacy message categories. The inline composer chip still renders
 * them for restored older drafts; nothing in the app creates new ones.
 */

import { t } from "../../../i18n";

export type ReportCategory = "bug" | "feedback" | "feature";

export type ReportCategoryDef = {
  key: ReportCategory;
  /** Chip label. Sentence case, no dashes (see CLAUDE.md). */
  label: string;
};

const REPORT_CATEGORIES: ReportCategoryDef[] = [
  {
    key: "bug",
    get label() {
      return t("chat.report.bug.label");
    },
  },
  {
    key: "feedback",
    get label() {
      return t("chat.report.feedback.label");
    },
  },
  {
    key: "feature",
    get label() {
      return t("chat.report.feature.label");
    },
  },
];

const BY_KEY = new Map<ReportCategory, ReportCategoryDef>(
  REPORT_CATEGORIES.map((category) => [category.key, category]),
);

export function reportCategoryDef(
  key: ReportCategory | string | null | undefined,
): ReportCategoryDef | undefined {
  if (!key) return undefined;
  return BY_KEY.get(key as ReportCategory);
}
