import { IconAnonymous } from "central-icons/IconAnonymous";
import { IconGhost2 } from "central-icons/IconGhost2";
import { IconLock } from "central-icons/IconLock";
import { modelPrivacyBadge, type ModelPrivacyBadge } from "../../lib/model-privacy";
import type { ProviderModelDto } from "../../lib/tauri";
import { HoverTip } from "./HoverTip";

/** The privacy mode's mascot: the lock for E2EE, the ghost for private, the
 * anonymized figure otherwise. Shared so every surface draws the same glyph
 * for a given mode. */
function privacyModeIcon(mode: ModelPrivacyBadge["mode"], size: number) {
  if (mode === "e2ee") return <IconLock size={size} aria-hidden />;
  if (mode === "private") return <IconGhost2 size={size} aria-hidden />;
  return <IconAnonymous size={size} aria-hidden />;
}

/**
 * The single chip for a model's privacy badge — the icon-by-mode (lock / ghost /
 * anonymous) plus its label. Shared by the model picker (`ModelMeta`), the chat
 * model hover cards (`ModelPickerCardContent`), the chat session bar
 * (`PrivacyModeBadge`), and the session usage panel so the surfaces stay
 * identical.
 *
 * `variant` picks the visual family: the default `"muted"` renders the quiet
 * `.model-trait-icon` chip; `"themed"` renders the brand-tinted pill
 * (`.agent-safety-badge`) the chat session bar uses. `size` shrinks the themed
 * pill: `"sm"` adds the `.agent-safety-badge-sm` modifier (a shorter pill for
 * dense rows like the usage panel's model row).
 *
 * `withTip` (default true) picks the wrapper: the tip variant wraps the chip in
 * a {@link HoverTip} carrying the badge's full description (focusable, with an
 * "label: description" aria-label) for standalone placements. Pass `withTip={false}`
 * when the chip already lives inside a hover card — a nested HoverTip would be
 * wrong there — to fall back to a plain span with a native `title` tooltip.
 *
 * `label` is an escape hatch for placements that already established a shorter
 * copy convention, while still sharing the same icon and chip styling.
 */
export function ModelPrivacyChip({
  badge,
  withTip = true,
  variant = "muted",
  size = "md",
  label = badge.label,
}: {
  badge: ModelPrivacyBadge;
  withTip?: boolean;
  variant?: "muted" | "themed";
  size?: "md" | "sm";
  label?: string;
}) {
  // Themed pills key off the safety-badge family; the muted chip stays on the
  // trait-icon recipe. Only the themed pill honors the small modifier.
  const className =
    variant === "themed"
      ? size === "sm"
        ? "agent-safety-badge agent-safety-badge-sm"
        : "agent-safety-badge"
      : "model-trait-icon";

  // The small themed pill drops the icon a notch to stay optically centered in
  // its shorter height; every other placement keeps the 13/14px icon.
  const iconSize = size === "sm" ? 12 : variant === "themed" ? 13 : 14;

  const icon = privacyModeIcon(badge.mode, iconSize);

  if (!withTip) {
    return (
      <span className={className} data-mode={badge.mode} title={badge.description}>
        {icon}
        <span>{label}</span>
      </span>
    );
  }

  return (
    <HoverTip
      tip={badge.description}
      className={className}
      data-mode={badge.mode}
      tabIndex={0}
      aria-label={`${badge.label}: ${badge.description}`}
    >
      {icon}
      <span>{label}</span>
    </HoverTip>
  );
}

/**
 * The compact private marker for model-picker rows. It stays icon-only because
 * row details live in the hover card. Anonymous/E2EE models keep their privacy
 * detail in the hover card instead of adding a trailing row icon.
 */
export function ModelRowPrivacyBadge({ model }: { model: ProviderModelDto }) {
  const badge = modelPrivacyBadge(model);
  if (badge?.mode !== "private") return null;
  return (
    <HoverTip
      tip={badge.description}
      className="model-row-privacy"
      data-mode={badge.mode}
      aria-label={`${badge.label}: ${badge.description}`}
    >
      {privacyModeIcon("private", 14)}
    </HoverTip>
  );
}
