import { IconCrossSmall } from "central-icons/IconCrossSmall";
import { IconLock } from "central-icons/IconLock";
import { useT } from "../../i18n";

export function PermissionBanner({
  onDismiss,
  onEnableAccessibility,
}: {
  onDismiss: () => void;
  onEnableAccessibility: () => void;
}) {
  const t = useT();
  return (
    <section className="message-card permission-banner" aria-label={t("shell.permission.label")}>
      <p className="permission-banner-message">
        <span className="permission-banner-eyebrow">
          <IconLock size={14} aria-hidden />
        </span>
        <span className="permission-banner-body">{t("shell.permission.body")}</span>
      </p>
      <div className="permission-banner-actions">
        <button type="button" className="btn btn-ghost" onClick={onEnableAccessibility}>
          {t("shell.permission.grant")}
        </button>
        <button
          type="button"
          className="permission-banner-dismiss"
          aria-label={t("shell.permission.dismissLabel")}
          title={t("shell.permission.dismiss")}
          onClick={onDismiss}
        >
          <IconCrossSmall size={14} aria-hidden />
        </button>
      </div>
    </section>
  );
}
