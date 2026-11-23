import { useEffect, useState } from "react";
import { browserAccessGet, browserAccessSet } from "./tauri";
import { messageFromError } from "./errors";
import { experimentalBrowserUseEnabled } from "./experimental-flags";

/** The literal token Clovy emits when a task needs Browser use while the
 * Browser access grant is off. The agent can never flip the setting itself:
 * the flag file lives outside every sandbox write root by design, so the
 * request is rendered as a card the user approves with one click. */
export const BROWSER_ACCESS_REQUEST_TOKEN = "[REQUEST:BROWSER_ACCESS]";

/** True when the text carries the request token and Browser use exists in
 * this build. While the feature flag is off no card must render (the setting
 * it would enable is hidden), but the strip below still runs so a stray
 * token never shows as literal text. */
export function hasBrowserAccessRequest(text: string) {
  return experimentalBrowserUseEnabled() && text.includes(BROWSER_ACCESS_REQUEST_TOKEN);
}

/** Removes the request token (and the blank line it sat on) from display
 * text; the card renders in its place. */
export function stripBrowserAccessRequest(text: string) {
  return text
    .split(BROWSER_ACCESS_REQUEST_TOKEN)
    .join("")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

/** A grant change updates every consent surface without restarting a turn. */
const BROWSER_ACCESS_CHANGED_EVENT = "clovy:browser-access-changed";

export function useBrowserAccess(active = true) {
  const [enabled, setEnabled] = useState<boolean>();
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string>();

  useEffect(() => {
    if (!active) return;
    let mounted = true;
    let revision = 0;
    const refresh = () => {
      const current = ++revision;
      void browserAccessGet().then(
        (next) => {
          if (mounted && current === revision) {
            setEnabled(next);
            setError(undefined);
          }
        },
        (cause) => {
          if (mounted && current === revision) setError(messageFromError(cause));
        },
      );
    };
    refresh();
    window.addEventListener(BROWSER_ACCESS_CHANGED_EVENT, refresh);
    window.addEventListener("focus", refresh);
    return () => {
      mounted = false;
      revision++;
      window.removeEventListener(BROWSER_ACCESS_CHANGED_EVENT, refresh);
      window.removeEventListener("focus", refresh);
    };
  }, [active]);

  async function setGrant(next: boolean) {
    if (submitting) return;
    setSubmitting(true);
    setError(undefined);
    try {
      setEnabled(await browserAccessSet(next));
      window.dispatchEvent(new Event(BROWSER_ACCESS_CHANGED_EVENT));
    } catch (cause) {
      setError(messageFromError(cause));
    } finally {
      setSubmitting(false);
    }
  }

  return { enabled, submitting, error, setGrant };
}
