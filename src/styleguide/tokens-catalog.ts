// Tolerant parser over the real tokens.css source. Groups tokens by the
// comment banners in that file (Type / Spacing / Radii / Controls / ... /
// Motion / Shadows) and captures the dark-theme overrides separately. Unknown
// lines are skipped; a token seen before any banner lands in "Other".
//
// resolveToken reads the LIVE computed value off :root, so swatches re-resolve
// when the theme or brand preset changes (color-mix tokens resolve to concrete
// colors, spacing/radius to px, etc.).

import tokensRaw from "../styles/tokens.css?raw";

export type Token = { name: string; rawValue: string };
export type TokenGroup = { group: string; tokens: Token[] };

// A banner is a SINGLE-LINE comment that opens and closes on the same line and
// carries a run of dashes, e.g. `/* Type ------------ */`. Requiring the
// closing `*/` on the same line keeps it from matching the first line of a
// multi-line comment whose prose happens to contain a `--token` reference.
const BANNER_RE = /^\/\*\s*([^-*][^-]*?)\s*-{2,}[^*]*\*\/\s*$/;
// A single-line custom-property declaration: `--name: value;`. Multi-line
// values (the shadow tokens) are handled by joining continuation lines first.
const DECL_RE = /^(--[\w-]+)\s*:\s*(.+?);\s*(?:\/\*.*)?$/;

// Human labels for the raw banner text, so the catalog reads cleanly.
const GROUP_LABELS: Record<string, string> = {
  Type: "Type",
  Spacing: "Spacing",
  Radii: "Radius",
  Controls: "Controls",
  "Sidebar dimensions": "Layout",
  "Color — light theme": "Color",
  Motion: "Motion",
  Shadows: "Elevation",
};

function labelFor(banner: string): string {
  return GROUP_LABELS[banner] ?? banner;
}

let cachedLight: TokenGroup[] | undefined;

function parse(): TokenGroup[] {
  const lines = tokensRaw.split("\n");

  const order: string[] = [];
  const byGroup = new Map<string, Token[]>();

  let currentGroup = "Other";
  let pending = ""; // accumulates a multi-line declaration
  let inComment = false; // inside a multi-line /* ... */ comment
  let depth = 0; // CSS brace nesting depth
  // Context recorded when a top-level (depth 0 -> 1) block opens: "root" is the
  // main :root catalog, "dark" is the dark override block, anything else is a
  // block whose declarations we ignore (@property, @media, transition selectors,
  // the light block). Only depth 1 inside "root"/"dark" yields catalog tokens.
  let topContext: "root" | "dark" | "other" = "other";
  // First declaration of a name wins. Dark overrides re-use the light token
  // names and are not part of the catalog.
  const seenLight = new Set<string>();

  const pushToken = (name: string, rawValue: string) => {
    if (topContext === "dark") return;
    // Light catalog: dedupe responsive @media re-declarations of the same name.
    if (seenLight.has(name)) return;
    seenLight.add(name);
    if (!byGroup.has(currentGroup)) {
      byGroup.set(currentGroup, []);
      order.push(currentGroup);
    }
    byGroup.get(currentGroup)?.push({ name, rawValue });
  };

  // Remember the selector text preceding an opening brace so we can classify
  // the block once we descend into it.
  let selector = "";

  for (const rawLine of lines) {
    let line = rawLine.trim();

    // Continue / close a multi-line comment.
    if (inComment) {
      const close = line.indexOf("*/");
      if (close === -1) continue;
      line = line.slice(close + 2).trim();
      inComment = false;
      if (line === "") continue;
    }

    // A banner (single-line `/* Label ---- */`) sets the group at catalog top.
    const banner = line.match(BANNER_RE);
    if (banner && depth === 1 && topContext === "root") {
      currentGroup = labelFor(banner[1].trim());
      pending = "";
      continue;
    }

    // Strip / open comments. The Color section has no single-line banner (its
    // header opens a multi-line comment: "Color — light theme..."), so treat
    // that opener as the Color group marker while inside the catalog root.
    if (!pending) {
      const open = line.indexOf("/*");
      if (open !== -1) {
        const close = line.indexOf("*/", open + 2);
        const body = line.slice(open + 2, close === -1 ? undefined : close).trim();
        if (body.startsWith("Color") && depth === 1 && topContext === "root") {
          currentGroup = "Color";
        }
        if (close === -1) {
          inComment = true;
          continue;
        }
        line = (line.slice(0, open) + line.slice(close + 2)).trim();
        if (line === "") continue;
      }
    }

    // Track opening braces and classify top-level blocks by their selector.
    if (line.includes("{")) {
      const head = line.slice(0, line.indexOf("{")).trim();
      selector = head || selector;
      depth += 1;
      if (depth === 1) {
        if (/^:root\b/.test(selector) && !selector.includes("[")) {
          topContext = "root";
        } else if (selector.startsWith('[data-theme="dark"]')) {
          topContext = "dark";
        } else {
          topContext = "other";
        }
      }
      selector = "";
      // A single-line `selector { ... }` is rare here; fall through so any
      // trailing `}` on the same line still decrements below.
      if (!line.includes("}")) continue;
    }

    if (line === "}") {
      if (depth > 0) depth -= 1;
      if (depth === 0) topContext = "other";
      pending = "";
      continue;
    }

    if (line === "") continue;

    // Only the main :root catalog and the dark block yield tokens; other
    // blocks (and anything at depth 0) are structural.
    if (depth !== 1 || topContext === "other") {
      // Remember a bare selector line (selector on its own, brace next line).
      if (!line.includes("{") && !line.startsWith("--") && !pending) selector = line;
      continue;
    }

    const candidate = pending ? `${pending} ${line}` : line;

    // A declaration that closes on this line (ends with `;`)?
    if (candidate.endsWith(";")) {
      const decl = candidate.match(DECL_RE);
      pending = "";
      if (decl) {
        pushToken(decl[1], decl[2].trim());
      }
      continue;
    }

    // Open declaration (`--name: ...` with no closing `;` yet) — start / keep
    // accumulating. Only begin accumulating on something that looks like a
    // declaration start, so stray lines don't get glued together.
    if (pending || /^--[\w-]+\s*:/.test(candidate)) {
      pending = candidate;
    }
  }

  // Drop empty groups and stabilize order.
  const light = order
    .map((group) => ({ group, tokens: byGroup.get(group) ?? [] }))
    .filter((g) => g.tokens.length > 0);

  return light;
}

export function getTokenGroups(): TokenGroup[] {
  if (!cachedLight) cachedLight = parse();
  return cachedLight;
}

// Live value off :root so swatches track theme + brand switches.
export function resolveToken(name: string): string {
  if (typeof document === "undefined") return "";
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}
