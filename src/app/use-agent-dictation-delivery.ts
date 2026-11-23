import { listen } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { toast } from "../components/ui/Toaster";
import { messageFromError } from "../lib/errors";
import { claimAgentDictationDelivery, dictationHelperCommand } from "../lib/tauri";
import { deliverAgentDictation } from "../lib/agent-session-drafts";

/** App-owned listener remains available when another view or partition is open. */
export function useAgentDictationDelivery() {
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<{
      takeId: string;
      composerRequestId: string;
      conversationId: string;
      profile: string;
      text: string;
    }>("clovy://agent-dictation-delivery", ({ payload }) => {
      if (disposed) return;
      void (async () => {
        await claimAgentDictationDelivery(payload.takeId, payload.composerRequestId);
        if (disposed) return;
        const ok = deliverAgentDictation(payload);
        await dictationHelperCommand({
          type: "agent_composer_delivered",
          takeId: payload.takeId,
          composerRequestId: payload.composerRequestId,
          ok,
        });
      })().catch((error) => toast.error(messageFromError(error)));
    }).then((cleanup) => {
      if (disposed) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);
}
