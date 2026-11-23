import { RUNTIME_BRAND_MARKS, type RuntimeBrand } from "./runtime-brand-marks";

/** A vendored one-color brand mark (src/assets/brand/), sized like a
 * central-icons glyph and painted with currentColor. */
export function BrandMark({ brand, size = 18 }: { brand: RuntimeBrand; size?: number }) {
  const mark = RUNTIME_BRAND_MARKS[brand];
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="currentColor"
      fillRule="evenodd"
      clipRule="evenodd"
      xmlns="http://www.w3.org/2000/svg"
      role="img"
      aria-label={mark.label}
      data-brand={brand}
      style={{ flex: "none" }}
    >
      {mark.paths.map((d) => (
        <path key={d} d={d} />
      ))}
    </svg>
  );
}

export function initials(source: string) {
  const cleaned = source
    .replace(/[/_-]+/g, " ")
    .replace(/[^A-Za-z0-9 ]/g, " ")
    .trim();
  if (!cleaned) return "";
  const words = cleaned.split(/\s+/);
  if (words.length === 1) return words[0].slice(0, 2).toUpperCase();
  return (words[0][0] + words[1][0]).toUpperCase();
}

/** Two-letter stand-in for a brand without an open-licensed mark; it keeps
 * the icon box so rows stay aligned. */
export function Monogram({ label, size }: { label: string; size: number }) {
  return (
    <span
      aria-hidden
      data-brand="monogram"
      style={{
        display: "inline-flex",
        flex: "none",
        alignItems: "center",
        justifyContent: "center",
        width: size,
        height: size,
        fontSize: Math.round(size * 0.55),
        fontWeight: "var(--fw-medium)",
        lineHeight: 1,
        letterSpacing: "-0.02em",
      }}
    >
      {label || "?"}
    </span>
  );
}
