import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import {
  listMcpExternalApprovals,
  MCP_EXTERNAL_APPROVALS_CHANGED,
  type McpExternalApproval,
} from "../../lib/mcp-server";

export function useMcpExternalApprovals() {
  const [requests, setRequests] = useState<McpExternalApproval[]>();
  const [unavailable, setUnavailable] = useState(false);
  const reload = useRef<() => Promise<void>>(async () => {});

  useEffect(() => {
    let disposed = false;
    let latest = 0;
    let connecting = false;
    let unlisten: (() => void) | undefined;

    async function read() {
      const version = ++latest;
      try {
        const next = await listMcpExternalApprovals();
        if (disposed || version !== latest) return;
        setRequests(next);
        setUnavailable(false);
      } catch {
        if (disposed || version !== latest) return;
        setRequests(undefined);
        setUnavailable(true);
      }
    }

    async function refresh() {
      if (disposed || connecting) return;
      if (!unlisten) {
        connecting = true;
        try {
          const cleanup = await listen(MCP_EXTERNAL_APPROVALS_CHANGED, () => void read());
          if (disposed) {
            cleanup();
            return;
          }
          unlisten = cleanup;
        } catch {
          if (!disposed) {
            setRequests(undefined);
            setUnavailable(true);
          }
          return;
        } finally {
          connecting = false;
        }
      }
      await read();
    }

    reload.current = refresh;
    void refresh();

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  return {
    requests,
    unavailable,
    refresh: () => reload.current(),
  };
}
