import type { ReactNode } from "react";
import type { MessageKey } from "../../i18n";
import { CODING_AGENT_LANE } from "../coding-agents/CodingAgentLane";

export type TimelineLaneProps = {
  /** The selected day's range, RFC 3339 (local midnight to next local midnight). */
  from: string;
  to: string;
  dayStartMs: number;
  dayEndMs: number;
  /** Returns a 0..1 fraction of the day for a timestamp in milliseconds. */
  xForTime(ms: number): number;
};

/**
 * Extension point for extra lanes drawn under the sessions lane on the same
 * 24-hour scale (for example the coding-agent lane). A lane fetches its own
 * data for `from`..`to` and positions items with `xForTime`.
 */
export type TimelineLane = {
  id: string;
  /** Catalog key of the lane's name (en and pt-BR). */
  label: MessageKey;
  render: (props: TimelineLaneProps) => ReactNode;
};

/** Registered extra lanes, rendered in order. */
export const EXTRA_TIMELINE_LANES: TimelineLane[] = [CODING_AGENT_LANE];
