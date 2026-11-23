import type { ReactNode } from "react";
import { useT } from "../../i18n";
import { BackButton } from "./BackButton";

type BreadcrumbItem = {
  label: string;
  /** Optional leading glyph (e.g. the project icon) for recognition. */
  icon?: ReactNode;
  onClick?: () => void;
  /** Optional action revealed beside the current crumb on hover or focus. */
  action?: ReactNode;
};

type Props = {
  // Back is the history affordance; omit it for surfaces you don't drill into
  // (the crumbs still read as the canonical location).
  backLabel?: string;
  onBack?: () => void;
  items: BreadcrumbItem[];
  actions?: ReactNode;
};

export function BreadcrumbBar({ backLabel, onBack, items, actions }: Props) {
  const t = useT();
  return (
    <div className="detail-bar" data-tauri-drag-region>
      {onBack ? <BackButton label={backLabel ?? t("common.back")} onClick={onBack} /> : null}
      <nav className="detail-breadcrumb" aria-label={t("shell.breadcrumb.label")}>
        <ol>
          {items.map((item, index) => {
            const current = index === items.length - 1;
            return (
              <li key={`${item.label}-${index}`}>
                {index > 0 ? (
                  <span className="detail-breadcrumb-separator" aria-hidden>
                    /
                  </span>
                ) : null}
                {item.onClick && !current ? (
                  <button type="button" className="detail-breadcrumb-link" onClick={item.onClick}>
                    {item.icon ? (
                      <span className="detail-breadcrumb-icon" aria-hidden>
                        {item.icon}
                      </span>
                    ) : null}
                    {item.label}
                  </button>
                ) : current && item.action ? (
                  <span className="detail-breadcrumb-current-group">
                    <span className="detail-breadcrumb-current">
                      {item.icon ? (
                        <span className="detail-breadcrumb-icon" aria-hidden>
                          {item.icon}
                        </span>
                      ) : null}
                      {item.label}
                    </span>
                    {item.action}
                  </span>
                ) : (
                  <span
                    className={current ? "detail-breadcrumb-current" : "detail-breadcrumb-label"}
                  >
                    {item.icon ? (
                      <span className="detail-breadcrumb-icon" aria-hidden>
                        {item.icon}
                      </span>
                    ) : null}
                    {item.label}
                  </span>
                )}
              </li>
            );
          })}
        </ol>
      </nav>
      {actions ? <div className="detail-bar-actions">{actions}</div> : null}
    </div>
  );
}
