import { IconShieldCheck } from "central-icons/IconShieldCheck";
import { useT } from "../../i18n";
import { useMcpExternalApprovals } from "./use-mcp-external-approvals";

export function McpApprovalIndicator({
  onOpen,
  floating = false,
}: {
  onOpen: () => void;
  floating?: boolean;
}) {
  const t = useT();
  const { requests } = useMcpExternalApprovals();
  const count = requests?.length ?? 0;
  if (count === 0) return null;

  const label = t("mcpServer.approvals.indicator", { count });
  return (
    <button
      type="button"
      className={floating ? "btn btn-secondary" : "sidebar-nav-item"}
      style={floating ? { alignSelf: "flex-end" } : undefined}
      onClick={onOpen}
      title={label}
      aria-label={label}
    >
      <span className="sidebar-nav-icon" aria-hidden>
        <IconShieldCheck size={16} />
      </span>
      <span className="sidebar-nav-label">{label}</span>
    </button>
  );
}
