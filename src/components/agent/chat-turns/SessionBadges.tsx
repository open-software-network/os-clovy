import { IconShieldCrossed } from "central-icons/IconShieldCrossed";
import { useT } from "../../../i18n";
import type { ModelPrivacyBadge } from "../../../lib/model-privacy";
import { ModelPrivacyChip } from "../../ui/ModelPrivacyChip";
import { HoverTip } from "../../ui/HoverTip";

export function PrivacyModeBadge({ badge }: { badge?: ModelPrivacyBadge }) {
  if (!badge) return null;
  // Delegates to the shared chip in the themed (brand-tinted pill) family so the
  // session bar and the usage panel render the same component. The look is
  // unchanged: themed-md keeps the 13px icon and the `.agent-safety-badge`
  // recipe; the aria-label now unifies to the shared "label: description" form.
  return <ModelPrivacyChip badge={badge} variant="themed" />;
}

// Indicator of the selected session's opt-in. The jail itself is
// per-process, but every send restarts the runtime into the target session's
// recorded mode, so the session — not the runtime's current state — is the
// honest unit to label.
export function UnrestrictedBadge() {
  const t = useT();
  const description = t("chat.sandbox.unrestrictedDescription");
  return (
    <HoverTip
      tip={description}
      className="agent-safety-badge agent-sandbox-badge"
      tabIndex={0}
      aria-label={t("chat.sandbox.unrestrictedAria", { description })}
    >
      <IconShieldCrossed size={13} aria-hidden />
      {t("chat.sandbox.unrestricted")}
    </HoverTip>
  );
}
