import { useRef, useState } from "react";
import { useT } from "../../i18n";
import { decideMcpExternalApproval } from "../../lib/mcp-server";
import { useMcpExternalApprovals } from "./use-mcp-external-approvals";

type Decision = "approved" | "denied" | "unavailable";

export function McpExternalApprovals() {
  const t = useT();
  const { requests, unavailable, refresh } = useMcpExternalApprovals();
  const [deciding, setDeciding] = useState<string>();
  const [decision, setDecision] = useState<Decision>();
  const heading = useRef<HTMLHeadingElement>(null);

  async function decide(id: string, approve: boolean) {
    setDeciding(id);
    setDecision(undefined);
    try {
      await decideMcpExternalApproval(id, approve);
      setDecision(approve ? "approved" : "denied");
    } catch {
      setDecision("unavailable");
    } finally {
      heading.current?.focus();
      await refresh();
      setDeciding(undefined);
    }
  }

  return (
    <section className="settings-group" aria-labelledby="mcp-external-approvals-heading">
      <h2
        ref={heading}
        tabIndex={-1}
        id="mcp-external-approvals-heading"
        className="settings-group-heading"
      >
        {t("mcpServer.approvals.heading")}
      </h2>
      <p className={decision ? "settings-group-description" : "visually-hidden"} role="status">
        {decision ? t(`mcpServer.approvals.${decision}`) : null}
      </p>
      {unavailable ? (
        <div className="settings-row">
          <p className="settings-row-error" role="alert">
            {t("mcpServer.approvals.loadError")}
          </p>
          <button type="button" className="btn btn-secondary" onClick={() => void refresh()}>
            {t("common.tryAgain")}
          </button>
        </div>
      ) : !requests?.length ? (
        <p className="settings-group-description">
          {t(requests ? "mcpServer.approvals.empty" : "mcpServer.approvals.loading")}
        </p>
      ) : (
        <div className="settings-card">
          <div className="settings-rows">
            {requests.map((request) => (
              <div key={request.id} className="settings-row settings-mcp-client">
                <div className="settings-row-info">
                  <h3 className="settings-row-title">{request.title}</h3>
                  <p className="settings-row-description">
                    {t("mcpServer.approvals.profile")} <bdi>{request.profile}</bdi>
                  </p>
                  <div className="settings-mcp-snippet">
                    <pre className="settings-mcp-snippet-code" dir="auto">
                      {request.preview}
                    </pre>
                  </div>
                </div>
                <div className="settings-row-control" aria-busy={deciding === request.id}>
                  <button
                    type="button"
                    className="btn btn-secondary"
                    disabled={deciding !== undefined}
                    onClick={() => void decide(request.id, false)}
                  >
                    {t("mcpServer.approvals.deny")}
                  </button>
                  <button
                    type="button"
                    className="btn btn-primary"
                    disabled={deciding !== undefined}
                    onClick={() => void decide(request.id, true)}
                  >
                    {t("mcpServer.approvals.approve")}
                  </button>
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
    </section>
  );
}
