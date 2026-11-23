import { IconAnthropic } from "central-icons/IconAnthropic";
import { IconClaudeai } from "central-icons/IconClaudeai";
import { IconDeepseek } from "central-icons/IconDeepseek";
import { IconGemini } from "central-icons/IconGemini";
import { IconGrok } from "central-icons/IconGrok";
import { IconMetaAi } from "central-icons/IconMetaAi";
import { IconMistral } from "central-icons/IconMistral";
import { IconNvidia } from "central-icons/IconNvidia";
import { IconOllama } from "central-icons/IconOllama";
import { IconOpenai } from "central-icons/IconOpenai";
import { IconPerplexity } from "central-icons/IconPerplexity";
import { BrandMark, Monogram, initials } from "../brand/BrandMark";
import { ClovyMark } from "../brand/ClovyLogo";

type ProviderLogoProps = {
  provider: string;
  id: string;
  name?: string;
  size?: number;
};

export function ProviderLogo({ provider, id, name = "", size = 18 }: ProviderLogoProps) {
  const kind = classifyProvider(provider, id, name);
  switch (kind) {
    case "clovy":
      return (
        <ClovyMark
          width={Math.round(size * 0.78)}
          height={Math.round(size * 0.78)}
          label="Clovy"
          variant="mono"
        />
      );
    case "openai":
      return <IconOpenai size={size} aria-label="OpenAI" />;
    case "anthropic":
      return <IconClaudeai size={size} aria-label="Claude" />;
    case "google":
      return <IconGemini size={size} aria-label="Gemini" />;
    case "meta":
      return <IconMetaAi size={size} aria-label="Meta" />;
    case "mistral":
      return <IconMistral size={size} aria-label="Mistral" />;
    case "deepseek":
      return <IconDeepseek size={size} aria-label="DeepSeek" />;
    case "perplexity":
      return <IconPerplexity size={size} aria-label="Perplexity" />;
    case "ollama":
      return <IconOllama size={size} aria-label="Ollama" />;
    case "xai":
      return <IconGrok size={size} aria-label="Grok" />;
    case "nvidia":
      return <IconNvidia size={size} aria-label="NVIDIA" />;
    case "elevenlabs":
      return <ElevenLabsMark size={size} />;
    case "fal":
      return <FalMark size={size} />;
    case "openrouter":
      return <BrandMark brand="openrouter" size={size} />;
    default:
      return <Monogram label={initials(name || id || provider)} size={size} />;
  }
}

// Anthropic gets two aliases — IconAnthropic and IconClaudeai. We prefer the
// product mark (Claude) because that's the visible product name.
export { IconAnthropic };

type ProviderKind =
  | "clovy"
  | "openai"
  | "anthropic"
  | "google"
  | "meta"
  | "mistral"
  | "deepseek"
  | "perplexity"
  | "ollama"
  | "xai"
  | "nvidia"
  | "elevenlabs"
  | "fal"
  | "openrouter"
  | "unknown";

function classifyProvider(provider: string, id: string, name: string): ProviderKind {
  // Auto is a Clovy-managed route even when its catalog record inherits a
  // provider value from the currently available routing backends.
  if (id.toLowerCase() === "open-software/auto") return "clovy";

  // Model family takes precedence over hosting platform (e.g. Groq-hosted
  // Whisper still reads as OpenAI). Hosting providers only win when nothing
  // more specific matched.
  const normalizedProvider = provider.toLowerCase();
  const haystack = `${normalizedProvider} ${id} ${name}`.toLowerCase();

  if (
    normalizedProvider === "openai" ||
    haystack.includes("gpt-") ||
    haystack.includes("whisper-") ||
    haystack.includes("o1-") ||
    haystack.includes("o3-") ||
    haystack.includes("o4-")
  ) {
    return "openai";
  }
  if (
    normalizedProvider === "anthropic" ||
    haystack.includes("claude") ||
    haystack.includes("anthropic")
  ) {
    return "anthropic";
  }
  if (haystack.includes("gemini") || haystack.includes("google")) {
    return "google";
  }
  if (haystack.includes("llama") || haystack.includes("meta-")) return "meta";
  if (haystack.includes("mistral") || haystack.includes("mixtral")) {
    return "mistral";
  }
  if (haystack.includes("deepseek")) return "deepseek";
  if (haystack.includes("perplexity") || haystack.includes("pplx")) {
    return "perplexity";
  }
  if (haystack.includes("ollama")) return "ollama";
  if (normalizedProvider === "xai" || haystack.includes("grok") || haystack.includes("xai/")) {
    return "xai";
  }
  if (haystack.includes("nvidia") || haystack.includes("parakeet")) {
    return "nvidia";
  }
  if (haystack.includes("eleven")) return "elevenlabs";

  if (
    normalizedProvider === "fal" ||
    haystack.includes("fal-ai/") ||
    haystack.includes("fal/") ||
    haystack.startsWith("fal ")
  ) {
    return "fal";
  }
  if (haystack.includes("openai")) return "openai";
  // The routing service itself, only once no model family matched.
  if (normalizedProvider === "openrouter") return "openrouter";

  return "unknown";
}

// Visual weight calibration. Custom SVGs fill their viewBox unevenly, so at
// equal pixel size their marks read larger or smaller than central-icons.
// These factors approximate the central-icons baseline (~85% viewBox fill).
const FAL_SCALE = 0.78;
const ELEVENLABS_SCALE = 1.05;

function ElevenLabsMark({ size }: { size: number }) {
  const s = Math.round(size * ELEVENLABS_SCALE);
  return (
    <svg
      width={s}
      height={s}
      viewBox="0 0 876 876"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      aria-label="ElevenLabs"
    >
      <path d="M468 292H528V584H468V292Z" fill="currentColor" />
      <path d="M348 292H408V584H348V292Z" fill="currentColor" />
    </svg>
  );
}

function FalMark({ size }: { size: number }) {
  const s = Math.round(size * FAL_SCALE);
  return (
    <svg
      width={s}
      height={s}
      viewBox="0 0 170 171"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      aria-label="Fal"
    >
      <path
        fillRule="evenodd"
        clipRule="evenodd"
        d="M109.571 0.690002C112.515 0.690002 114.874 3.08348 115.155 6.01352C117.665 32.149 138.466 52.948 164.603 55.458C167.534 55.7394 169.927 58.0985 169.927 61.042V110.255C169.927 113.198 167.534 115.557 164.603 115.839C138.466 118.349 117.665 139.148 115.155 165.283C114.874 168.213 112.515 170.607 109.571 170.607H60.3553C57.4116 170.607 55.0524 168.213 54.7709 165.283C52.2608 139.148 31.4601 118.349 5.32289 115.839C2.39266 115.557 -0.000976562 113.198 -0.000976562 110.255V61.042C-0.000976562 58.0985 2.39267 55.7394 5.3229 55.458C31.4601 52.948 52.2608 32.149 54.7709 6.01351C55.0524 3.08348 57.4116 0.690002 60.3553 0.690002H109.571ZM34.1182 85.5045C34.1182 113.776 57.0124 136.694 85.2539 136.694C113.495 136.694 136.39 113.776 136.39 85.5045C136.39 57.2332 113.495 34.3147 85.2539 34.3147C57.0124 34.3147 34.1182 57.2332 34.1182 85.5045Z"
        fill="currentColor"
      />
    </svg>
  );
}
