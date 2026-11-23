import { useT } from "../../i18n";
import { isWindowsPlatform } from "../../lib/platform";

/**
 * Renders a keyboard shortcut as platform-appropriate keycaps inside a single
 * framed group. Shared by Settings and the Dictation page.
 */
const MAC_KEY_GLYPHS: Record<string, { glyph: string; name: string }> = {
  cmd: { glyph: "⌘", name: "Command" },
  command: { glyph: "⌘", name: "Command" },
  meta: { glyph: "⌘", name: "Command" },
  ctrl: { glyph: "⌃", name: "Control" },
  control: { glyph: "⌃", name: "Control" },
  opt: { glyph: "⌥", name: "Option" },
  option: { glyph: "⌥", name: "Option" },
  alt: { glyph: "⌥", name: "Option" },
  shift: { glyph: "⇧", name: "Shift" },
  fn: { glyph: "fn", name: "Function" },
  function: { glyph: "fn", name: "Function" },
};

const WINDOWS_KEY_GLYPHS: Record<string, { glyph: string; name: string }> = {
  cmd: { glyph: "Win", name: "Windows" },
  command: { glyph: "Win", name: "Windows" },
  meta: { glyph: "Win", name: "Windows" },
  win: { glyph: "Win", name: "Windows" },
  ctrl: { glyph: "Ctrl", name: "Control" },
  control: { glyph: "Ctrl", name: "Control" },
  opt: { glyph: "Alt", name: "Alt" },
  option: { glyph: "Alt", name: "Alt" },
  alt: { glyph: "Alt", name: "Alt" },
  shift: { glyph: "Shift", name: "Shift" },
};

/** Lone keys a Windows shortcut can be (labels as the helper writes them),
 * named in the interface language. */
const WINDOWS_LONE_KEY_NAMES = {
  "right ctrl": "shell.shortcut.rightCtrl",
  "right alt": "shell.shortcut.rightAlt",
  "right shift": "shell.shortcut.rightShift",
  "left ctrl": "shell.shortcut.leftCtrl",
  "left alt": "shell.shortcut.leftAlt",
  "left shift": "shell.shortcut.leftShift",
} as const;

export function KeycapShortcut({
  label,
  capturing = false,
  platform,
}: {
  label: string;
  capturing?: boolean;
  platform?: "macos" | "windows" | "unsupported";
}) {
  const t = useT();
  if (capturing) {
    return (
      <span className="keycap-frame keycap-frame-capturing">
        {t("shell.shortcut.pressShortcut")}
      </span>
    );
  }
  const windows = platform === "windows" || (platform === undefined && isWindowsPlatform());
  const keyGlyphs = windows ? WINDOWS_KEY_GLYPHS : MAC_KEY_GLYPHS;
  const keys = label.split("+").filter(Boolean);
  return (
    <span className="keycap-frame" aria-label={t("shell.shortcut.ariaLabel", { label })}>
      {keys.map((key, idx) => {
        const lone = windows
          ? WINDOWS_LONE_KEY_NAMES[key.toLowerCase() as keyof typeof WINDOWS_LONE_KEY_NAMES]
          : undefined;
        const mapped = lone ? { glyph: t(lone), name: t(lone) } : keyGlyphs[key.toLowerCase()];
        return (
          <kbd
            key={`${key}-${idx}`}
            className="keycap"
            title={mapped?.name === "Function" ? t("shell.shortcut.keyFunction") : mapped?.name}
          >
            {mapped ? mapped.glyph : key}
          </kbd>
        );
      })}
    </span>
  );
}
