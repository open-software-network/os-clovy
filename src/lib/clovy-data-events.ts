import { listen } from "@tauri-apps/api/event";
import { useEffect, useLayoutEffect, useRef } from "react";
import { getCurrentDataPartitionName } from "./data-partition";

export type ClovyDataChange = {
  profile: string;
  kind: string;
  id?: string;
  deleted?: boolean;
};

/** Native mutations invalidate the same data the visible surface reads.
 * Check isCurrent after asynchronous reads so a partition switch cannot
 * hydrate the new view with the previous partition's response. */
export function useClovyDataChanged(
  kind: string,
  refresh: (change: ClovyDataChange, isCurrent: () => boolean) => void | Promise<void>,
  installation = false,
) {
  const refreshRef = useRef(refresh);
  useLayoutEffect(() => {
    refreshRef.current = refresh;
  }, [refresh]);
  useEffect(() => {
    let disposed = false;
    let sequence = 0;
    let unlisten: (() => void) | undefined;
    void listen<ClovyDataChange>("clovy-data-changed", ({ payload }) => {
      if (payload.kind !== kind) return;
      if (!installation && payload.profile !== getCurrentDataPartitionName()) return;
      const request = ++sequence;
      const isCurrent = () =>
        !disposed &&
        request === sequence &&
        (installation || payload.profile === getCurrentDataPartitionName());
      void refreshRef.current(payload, isCurrent);
    })
      .then((cleanup) => {
        if (disposed) cleanup();
        else unlisten = cleanup;
      })
      .catch(() => {
        if (!disposed) console.warn("Clovy data refresh subscription is unavailable.");
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [kind, installation]);
}
