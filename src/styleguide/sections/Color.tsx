import { resolveToken } from "../tokens-catalog";

// Color tokens grouped by role. Brand-derived tokens sit in their own group so
// switching the brand preset visibly re-tints them.
const COLOR_GROUPS: { label: string; names: string[] }[] = [
  {
    label: "Background and surface",
    names: [
      "--background",
      "--sidebar",
      "--card",
      "--popover",
      "--secondary",
      "--muted",
      "--accent",
      "--surface-subtle",
    ],
  },
  {
    label: "Text",
    names: [
      "--foreground",
      "--card-foreground",
      "--muted-foreground",
      "--body-copy",
      "--spinner-neutral",
    ],
  },
  {
    label: "Accent state",
    names: ["--primary", "--primary-foreground"],
  },
  {
    label: "Primary actions",
    names: [
      "--primary-action-background",
      "--primary-action-background-hover",
      "--primary-action-border",
      "--primary-action-foreground",
    ],
  },
  {
    label: "Border and ring",
    names: [
      "--border",
      "--border-subtle",
      "--detail-bar-border",
      "--input",
      "--ring",
      "--ring-focus",
      "--focus-ring",
    ],
  },
  {
    label: "Brand and derived tints",
    names: [
      "--brand",
      "--brand-bright",
      "--brand-wash",
      "--brand-tint",
      "--brand-tint-strong",
      "--brand-line",
      "--brand-line-strong",
      "--warm-soft",
      "--warm-strong",
      "--agent-background-ink",
    ],
  },
  {
    label: "Status",
    names: ["--destructive", "--destructive-soft", "--success", "--record"],
  },
];

function Swatch({ name }: { name: string }) {
  return (
    <div>
      <div className="sg-swatch-block" style={{ background: `var(${name})` }} />
      <div className="sg-token-meta">
        <span className="sg-token-name">{name}</span>
        <span className="sg-token-value">{resolveToken(name)}</span>
      </div>
    </div>
  );
}

export function Color() {
  return (
    <div className="sg-section">
      <h1 className="sg-section-heading">Color</h1>
      <p className="sg-section-intro">
        Every surface, text, and edge color is a theme-aware token. The brand-derived tints mix from
        the selected accent, so switching a brand preset above re-tints them live without touching
        the neutral text tokens.
      </p>
      {COLOR_GROUPS.map((group) => (
        <div key={group.label}>
          <h2 className="sg-subheading">{group.label}</h2>
          <div className="sg-grid">
            {group.names.map((name) => (
              <Swatch key={name} name={name} />
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}
