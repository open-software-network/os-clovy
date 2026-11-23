import { IconApps } from "central-icons/IconApps";
import { useEffect, useState, type ReactNode } from "react";
import { activityAppIcon } from "../../lib/activity-timeline";

type AppIconProps = {
  /** Identity of the app; without one there is nothing to look up. */
  bundleId: string | null;
  size?: number;
  fallback?: ReactNode;
};

/**
 * The installed app's real icon, read on this Mac by bundle id. A neutral
 * glyph stands in while it loads and when the app or its icon is gone, never
 * another app's icon. Decorative: the app's name is always written next to it.
 */
export function AppIcon({ bundleId, size = 20, fallback }: AppIconProps) {
  const [resolved, setResolved] = useState<{ bundleId: string; src: string | null } | null>(null);

  useEffect(() => {
    if (!bundleId) return;
    let active = true;
    void activityAppIcon(bundleId).then((src) => {
      if (active) setResolved({ bundleId, src });
    });
    return () => {
      active = false;
    };
  }, [bundleId]);

  const src = bundleId && resolved?.bundleId === bundleId ? resolved.src : null;
  if (src && bundleId) {
    return (
      <img
        className="app-icon"
        src={src}
        alt=""
        width={size}
        height={size}
        onError={() =>
          setResolved((current) =>
            current?.bundleId === bundleId ? { bundleId, src: null } : current,
          )
        }
      />
    );
  }
  return (
    <span
      className="app-icon app-icon-fallback"
      data-testid="app-icon-fallback"
      aria-hidden="true"
      style={{ width: size, height: size }}
    >
      {fallback ?? <IconApps size={Math.round(size * 0.7)} />}
    </span>
  );
}
