import type { ConnectorPolicyCatalog, RoutineTrustMode } from "../../lib/tauri";
import {
  autonomyProgressLabel,
  autonomyUnlockHint,
  canSelectAutonomous,
  TRUST_MODE_META,
  grantableConnectorActionTools,
} from "../../lib/connectors";
import { Checkbox } from "../ui/Checkbox";
import { useT } from "../../i18n";
import { SegmentedControl } from "../ui/SegmentedControl";

const TRUST_MODES: readonly RoutineTrustMode[] = ["read_only", "approval", "autonomous"];

// Built per render: the labels follow the interface language.
function modeOptions() {
  return TRUST_MODES.map((mode) => {
    const meta = TRUST_MODE_META[mode];
    const Icon = meta.icon;
    return {
      value: mode,
      label: (
        <>
          <Icon size={14} aria-hidden />
          {meta.label}
        </>
      ),
      ariaLabel: meta.label,
    };
  });
}

/**
 * The per-routine trust choice for connector actions (read only, approval,
 * autonomous). Deliberately SEPARATE from RoutineModePicker: trust governs
 * what a routine may do with your Google account, the sandbox choice governs
 * what it may do to your machine — the two never merge.
 *
 * Autonomous is earned: it stays unselectable (the pick is ignored, with the
 * unlock hint explaining why) until `runCount` approval-mode runs completed.
 * When autonomous is active, the checklist selects exactly which connector
 * action tools the grant covers.
 */
export function TrustModePicker({
  value,
  policy,
  runCount,
  autonomousTools,
  onChange,
  onAutonomousToolsChange,
}: {
  value: RoutineTrustMode;
  policy: ConnectorPolicyCatalog | null;
  /** Completed approval-mode runs of this routine (0 for a new one). */
  runCount: number;
  /** Connector action tool names granted for autonomous runs. */
  autonomousTools: string[];
  onChange: (mode: RoutineTrustMode) => void;
  onAutonomousToolsChange: (tools: string[]) => void;
}) {
  const t = useT();
  const grantableTools = policy ? grantableConnectorActionTools(policy) : [];
  const autonomousUnlocked = policy ? canSelectAutonomous(policy, runCount) : false;

  function pick(mode: RoutineTrustMode) {
    if (mode === "autonomous" && !autonomousUnlocked) return;
    onChange(mode);
  }

  function toggleTool(toolId: string, granted: boolean) {
    const next = new Set(autonomousTools);
    if (granted) next.add(toolId);
    else next.delete(toolId);
    onAutonomousToolsChange(grantableTools.map((tool) => tool.id).filter((id) => next.has(id)));
  }

  return (
    <div className="trust-mode-picker">
      <SegmentedControl
        value={value}
        onValueChange={pick}
        options={modeOptions()}
        aria-label={t("routines.trust.question")}
      />
      <p className="routines-mode-hint">{TRUST_MODE_META[value].description}</p>
      {policy && !autonomousUnlocked ? (
        <p className="routines-mode-hint trust-mode-locked-hint">
          {autonomyUnlockHint(policy, runCount)} {autonomyProgressLabel(policy, runCount)}
        </p>
      ) : null}
      {value === "autonomous" ? (
        <fieldset className="trust-mode-grants">
          <legend className="trust-mode-grants-legend">{t("routines.trust.grantsLegend")}</legend>
          {/* Grantable tools only: Linear's write tools are approval-only in v1
              and must never appear as autonomy checkboxes. */}
          {grantableTools.map((tool) => (
            <label key={tool.id} className="trust-mode-grant" htmlFor={`trust-grant-${tool.id}`}>
              <Checkbox
                id={`trust-grant-${tool.id}`}
                checked={autonomousTools.includes(tool.id)}
                onChange={(event) => toggleTool(tool.id, event.currentTarget.checked)}
              />
              {tool.label}
            </label>
          ))}
        </fieldset>
      ) : null}
    </div>
  );
}
