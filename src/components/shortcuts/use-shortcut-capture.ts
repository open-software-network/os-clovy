import type { DictationShortcutSetting } from "../../lib/tauri";
import { isWindowsPlatform } from "../../lib/platform";

export type CapturedShortcut = Pick<
  DictationShortcutSetting,
  "code" | "modifiers" | "label" | "pressCount"
>;

export function shortcutFromCapturePayload(
  shortcut: unknown,
  fallbackPressCount: 1 | 2,
): CapturedShortcut | undefined {
  if (!shortcut || typeof shortcut !== "object") return undefined;

  const value = shortcut as Partial<DictationShortcutSetting>;
  const modifiers = value.modifiers;
  const pressCount =
    value.pressCount === 1 || value.pressCount === 2 ? value.pressCount : fallbackPressCount;
  if (
    typeof value.code !== "string" ||
    typeof value.label !== "string" ||
    !modifiers ||
    typeof modifiers.command !== "boolean" ||
    typeof modifiers.control !== "boolean" ||
    typeof modifiers.option !== "boolean" ||
    typeof modifiers.shift !== "boolean" ||
    typeof modifiers.function !== "boolean"
  ) {
    return undefined;
  }

  return {
    code: value.code,
    label: value.label,
    modifiers,
    pressCount,
  };
}

/** What a keydown means for an in-progress capture. Bare modifier presses
 * are ignored here (the helper's flagsChanged monitor owns those, since the
 * DOM cannot see fn); a real key either completes a chord or, without any
 * modifier, asks the user for one. */
export type CaptureKeyResult =
  | { kind: "ignore" }
  | { kind: "needsModifier" }
  | { kind: "chord"; shortcut: CapturedShortcut };

export function chordFromKeyEvent(event: KeyboardEvent): CaptureKeyResult {
  if (["Shift", "Control", "Alt", "Meta"].includes(event.key)) {
    return { kind: "ignore" };
  }
  if (!event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey) {
    return { kind: "needsModifier" };
  }
  const windows = isWindowsPlatform();
  const modifiers = {
    command: event.metaKey,
    control: event.ctrlKey,
    option: event.altKey,
    shift: event.shiftKey,
    function: false,
  };
  const label = [
    modifiers.command ? (windows ? "Win" : "Cmd") : undefined,
    modifiers.control ? "Ctrl" : undefined,
    modifiers.option ? (windows ? "Alt" : "Opt") : undefined,
    modifiers.shift ? "Shift" : undefined,
    keyLabel(event.code),
  ]
    .filter((part): part is string => Boolean(part))
    .join("+");
  return {
    kind: "chord",
    shortcut: { code: event.code, modifiers, label, pressCount: 1 },
  };
}

/** Friendly key name from a DOM `code`: "KeyD" -> "D", "Digit5" -> "5";
 * anything else (F5, Space, ArrowUp) already reads fine as-is. */
function keyLabel(code: string) {
  if (code.startsWith("Key") && code.length === 4) return code.slice(3);
  if (code.startsWith("Digit") && code.length === 6) return code.slice(5);
  return code;
}
