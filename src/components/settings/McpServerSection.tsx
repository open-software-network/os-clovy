import { writeText as writeClipboardText } from "@tauri-apps/plugin-clipboard-manager";
import { useEffect, useRef, useState } from "react";
import { t as translate, useT } from "../../i18n";
import type { MessageKey } from "../../i18n/catalog";
import {
  type McpServerStatusDto,
  mcpServerSetEnabled,
  mcpServerStatus,
} from "../../lib/mcp-server";
import { CopyStateIcon } from "../ui/CopyStateIcon";
import { Switch } from "../ui/Switch";
import { RuntimeLogo } from "../brand/RuntimeLogo";
import { McpExternalApprovals } from "./McpExternalApprovals";

type SnippetId = "claudeCodeCommand" | "claudeCodeConfig" | "cursorConfig";

const COPIED_RESET_MS = 1600;

/** MCP settings: the Clovy server switch, external approvals and configuration to
 * paste into MCP clients. Hidden where the server is unsupported. */
export function McpServerSection() {
  const t = useT();
  const [status, setStatus] = useState<McpServerStatusDto>();
  const [saving, setSaving] = useState(false);
  const [copied, setCopied] = useState<SnippetId>();
  const [error, setError] = useState<string>();
  const copiedTimer = useRef<number>(undefined);

  useEffect(() => {
    void mcpServerStatus()
      .then(setStatus)
      .catch((cause) => setError(messageFromError(cause)));
    return () => window.clearTimeout(copiedTimer.current);
  }, []);

  async function changeEnabled(enabled: boolean) {
    setSaving(true);
    setError(undefined);
    try {
      setStatus(await mcpServerSetEnabled(enabled));
    } catch (cause) {
      setError(messageFromError(cause));
    } finally {
      setSaving(false);
    }
  }

  async function copy(id: SnippetId, text: string) {
    try {
      await writeClipboardText(text);
      setCopied(id);
      window.clearTimeout(copiedTimer.current);
      copiedTimer.current = window.setTimeout(() => setCopied(undefined), COPIED_RESET_MS);
    } catch (cause) {
      setError(messageFromError(cause));
    }
  }

  if (!status?.supported) return null;

  const snippet = (id: SnippetId, text: string, ariaKey: MessageKey) => (
    <div className="settings-mcp-snippet">
      <pre className="settings-mcp-snippet-code">{text}</pre>
      <button
        type="button"
        className="btn btn-secondary settings-mcp-snippet-copy"
        aria-label={copied === id ? t("mcpServer.copied") : t(ariaKey)}
        data-copied={copied === id ? "true" : undefined}
        onClick={() => void copy(id, text)}
      >
        <CopyStateIcon copied={copied === id} />
        {copied === id ? t("mcpServer.copied") : t("mcpServer.copy")}
      </button>
    </div>
  );

  return (
    <>
      {status.enabled ? <McpExternalApprovals /> : null}
      <section className="settings-group" aria-labelledby="mcp-server-heading">
        <h2 id="mcp-server-heading" className="settings-group-heading">
          {t("mcpServer.heading")}
        </h2>
        <p className="settings-group-description">{t("mcpServer.description")}</p>
        <div className="settings-card">
          <div className="settings-rows">
            <div className="settings-row">
              <div className="settings-row-info">
                <h3 className="settings-row-title">{t("mcpServer.toggleTitle")}</h3>
                <p className="settings-row-description">
                  {status.enabled && status.running
                    ? t("mcpServer.running")
                    : t("mcpServer.toggleDescription")}
                </p>
              </div>
              <div className="settings-row-control">
                <Switch
                  checked={status.enabled}
                  disabled={saving}
                  onCheckedChange={(enabled) => void changeEnabled(enabled)}
                  aria-label={t("mcpServer.toggleAria")}
                />
              </div>
            </div>
            {status.enabled ? (
              <>
                <div className="settings-row settings-mcp-client">
                  <div className="settings-row-info">
                    <h3 className="settings-row-title settings-row-title-with-mark">
                      <RuntimeLogo id="claude" size={16} decorative />
                      {t("mcpServer.claudeCode.title")}
                    </h3>
                    <p className="settings-row-description">
                      {t("mcpServer.claudeCode.description")}
                    </p>
                    {snippet(
                      "claudeCodeCommand",
                      status.claudeCodeCommand,
                      "mcpServer.copyCommand",
                    )}
                    {snippet(
                      "claudeCodeConfig",
                      status.claudeCodeConfig,
                      "mcpServer.copyClaudeCodeConfig",
                    )}
                  </div>
                </div>
                <div className="settings-row settings-mcp-client">
                  <div className="settings-row-info">
                    <h3 className="settings-row-title settings-row-title-with-mark">
                      <RuntimeLogo id="cursor" size={16} decorative />
                      {t("mcpServer.cursor.title")}
                    </h3>
                    <p className="settings-row-description">{t("mcpServer.cursor.description")}</p>
                    {snippet("cursorConfig", status.cursorConfig, "mcpServer.copyCursorConfig")}
                  </div>
                </div>
              </>
            ) : null}
          </div>
        </div>
        {status.enabled && !status.binaryFound ? (
          <p className="settings-row-error" role="alert">
            {t("mcpServer.binaryMissing")}
          </p>
        ) : null}
        {status.error ? (
          <p className="settings-row-error" role="alert">
            {t("mcpServer.startError", { message: status.error })}
          </p>
        ) : null}
        {error ? (
          <p className="settings-row-error" role="alert">
            {error}
          </p>
        ) : null}
      </section>
    </>
  );
}

function messageFromError(error: unknown) {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  if (error && typeof error === "object" && "message" in error) {
    return String(error.message);
  }
  return translate("mcpServer.updateError");
}
