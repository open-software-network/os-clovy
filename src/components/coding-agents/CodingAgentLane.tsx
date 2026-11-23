import { useEffect, useState } from "react";
import { useLocale, useT } from "../../i18n";
import {
  CODING_AGENT_NAMES,
  type CodingAgentBlockDto,
  codingAgentBlocks,
  onCodingAgentsUpdated,
} from "../../lib/coding-agents";
import type { TimelineLane, TimelineLaneProps } from "../activity-timeline/lanes";
import { formatTime } from "../activity-timeline/timeline-utils";
import { HoverTip } from "../ui/HoverTip";
import { RuntimeLogo } from "../brand/RuntimeLogo";

/** Narrowest drawn block, as a fraction of the day, so short blocks stay hoverable. */
const MIN_BLOCK_FRACTION = 0.0025;

/**
 * The coding-agent lane of the "Today" view: one block per coding-agent block
 * of the selected day, placed on the shared 24 h scale. Hover or focus shows
 * agent, project, time, prompts, and summary. Loads its own data and reloads
 * when the scanner reports new blocks or summaries.
 */
export function CodingAgentLane({ from, to, xForTime }: TimelineLaneProps) {
  const t = useT();
  const locale = useLocale();
  const [blocks, setBlocks] = useState<CodingAgentBlockDto[]>([]);

  useEffect(() => {
    let active = true;
    const refresh = () => {
      void codingAgentBlocks(new Date(from), new Date(to))
        .then((next) => {
          if (active) setBlocks(next);
        })
        .catch(() => {});
    };
    refresh();
    let unlisten: (() => void) | undefined;
    void onCodingAgentsUpdated(refresh).then((cleanup) => {
      if (active) unlisten = cleanup;
      else cleanup();
    });
    return () => {
      active = false;
      unlisten?.();
    };
  }, [from, to]);

  if (blocks.length === 0) {
    return <p className="coding-agent-lane-empty">{t("codingAgents.lane.empty")}</p>;
  }

  return (
    <ul className="coding-agent-lane" aria-label={t("codingAgents.lane.listAria")}>
      {blocks.map((block) => {
        const left = xForTime(new Date(block.startedAt).getTime());
        const right = xForTime(new Date(block.endedAt).getTime());
        const agent = CODING_AGENT_NAMES[block.source];
        const project = block.project ?? t("codingAgents.lane.noProject");
        const time = t("codingAgents.lane.timeRange", {
          start: formatTime(block.startedAt, locale),
          end: formatTime(block.endedAt, locale),
        });
        return (
          <li
            key={block.id}
            className="coding-agent-lane-block"
            data-state={block.state}
            style={{
              left: `${left * 100}%`,
              width: `${Math.max(MIN_BLOCK_FRACTION, right - left) * 100}%`,
            }}
          >
            <HoverTip
              className="coding-agent-lane-anchor"
              tip={
                <div className="coding-agent-lane-tip">
                  <div className="coding-agent-lane-tip-meta">
                    <span className="coding-agent-lane-tip-agent">
                      <RuntimeLogo id={block.source} name={agent} size={14} decorative />
                      {agent}
                    </span>
                    <span>{project}</span>
                    <span>{time}</span>
                    <span>{t("codingAgents.lane.prompts", { count: block.promptCount })}</span>
                  </div>
                  {block.summary ? (
                    <p className="coding-agent-lane-tip-summary">{block.summary}</p>
                  ) : (
                    <>
                      <p className="coding-agent-lane-tip-status">
                        {block.state === "live"
                          ? t("codingAgents.lane.live")
                          : t("codingAgents.lane.waiting")}
                      </p>
                      {block.firstPrompt ? (
                        <p className="coding-agent-lane-tip-prompt">{block.firstPrompt}</p>
                      ) : null}
                    </>
                  )}
                </div>
              }
            >
              <button
                type="button"
                className="coding-agent-lane-button"
                aria-label={`${agent}, ${project}, ${time}`}
              />
            </HoverTip>
          </li>
        );
      })}
    </ul>
  );
}

/** Registered in `EXTRA_TIMELINE_LANES`. */
export const CODING_AGENT_LANE: TimelineLane = {
  id: "coding-agents",
  label: "codingAgents.lane.title",
  render: (props) => <CodingAgentLane {...props} />,
};
