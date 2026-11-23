import { ProviderLogo } from "../settings/ProviderLogo";
import type { RuntimeBrand } from "./runtime-brand-marks";
import { BrandMark } from "./BrandMark";

// Agent CLI ids (src-tauri/src/llm/cli.rs), coding-agent source ids
// (src/lib/coding-agents.ts) and OpenRouter, by the mark they show.
const RUNTIME_BRANDS: Record<string, RuntimeBrand> = {
  claude: "claudecode",
  claude_code: "claudecode",
  codex: "codex",
  copilot: "githubcopilot",
  copilot_cli: "githubcopilot",
  copilot_vscode: "githubcopilot",
  "cursor-agent": "cursor",
  cursor: "cursor",
  cursor_cli: "cursor",
  agy: "antigravity",
  antigravity: "antigravity",
  pi: "pi",
  openrouter: "openrouter",
};

export function runtimeBrand(id: string): RuntimeBrand | null {
  return RUNTIME_BRANDS[id.toLowerCase()] ?? null;
}

type RuntimeLogoProps = {
  /** CLI id, coding-agent source id, or provider id. */
  id: string;
  /** Display name, used for the monogram when no mark exists. */
  name?: string;
  size?: number;
  /** Beside a visible name: hidden from assistive tech so it is not read twice. */
  decorative?: boolean;
};

/** Brand mark of an agent runtime or provider; anything else falls back to
 * the provider marks and then to a monogram. */
export function RuntimeLogo({ id, name = "", size = 18, decorative = false }: RuntimeLogoProps) {
  const brand = runtimeBrand(id);
  const logo = brand ? (
    <BrandMark brand={brand} size={size} />
  ) : (
    <ProviderLogo provider={id} id={id} name={name} size={size} />
  );
  if (!decorative) return logo;
  return (
    <span className="runtime-logo" aria-hidden>
      {logo}
    </span>
  );
}
