import { afterEach, describe, expect, it } from "vitest";
import { applyBrandVar, BRAND_PRESETS } from "../lib/brand";
import appCss from "../styles/app.css?raw";
import tokensCss from "../styles/tokens.css?raw";

function cssRuleFor(selector: string, css = appCss) {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = new RegExp(`(?:^|\\n)${escaped}\\s*\\{`).exec(css);
  if (!match) throw new Error(`Missing CSS rule for ${selector}`);
  const openIndex = match.index + match[0].length - 1;
  let depth = 0;
  for (let index = openIndex; index < css.length; index += 1) {
    if (css[index] === "{") depth += 1;
    if (css[index] === "}") {
      depth -= 1;
      if (depth === 0) return css.slice(openIndex + 1, index);
    }
  }
  throw new Error(`Unclosed CSS rule for ${selector}`);
}

describe("Home polish styles", () => {
  it("keeps the hero as compact as the routine composer while allowing draft growth", () => {
    expect(appCss).not.toMatch(/\.agent-composer\[data-hero="true"\] \.agent-composer-editor\s*\{/);
    const editor = cssRuleFor(".agent-composer-editor");
    const routine = cssRuleFor(".routines-describe-composer textarea");
    for (const rule of [editor, routine]) {
      expect(rule).toContain("min-height: var(--control-lg);");
      expect(rule).toContain("padding: 6px var(--sp-5);");
      expect(rule).toContain("font-size: var(--fs-lg);");
    }
    expect(editor).toContain("max-height: 200px;");
    expect(appCss).not.toContain(".agent-approval-control");
  });

  it("lets the background surfaces fit a narrow window without clipping the hero controls", () => {
    expect(cssRuleFor('body[data-agent-surface="true"]')).toContain("min-width: 0;");
    expect(cssRuleFor(".agent-hero-chips")).toContain("flex-wrap: wrap;");
    expect(cssRuleFor(".agent-hero-title")).toContain("text-align: center;");
  });
  it("gives the Home thread room at the top edge", () => {
    const timeline = cssRuleFor('.agent-timeline[data-home="true"]');

    expect(timeline).toContain("padding-top: var(--sp-8);");
  });

  it("aligns idle suggestions with the composer and keeps the pending mark bare", () => {
    const nudges = cssRuleFor(".agent-home-nudges-idle");
    const pending = cssRuleFor(".agent-home-task-pending");

    expect(nudges).toContain("padding-inline: 0;");
    expect(pending).not.toMatch(/^\s*width:/m);
    expect(pending).not.toMatch(/^\s*height:/m);
    expect(pending).not.toMatch(/^\s*padding:/m);
    expect(pending).toContain("justify-self: start;");
  });

  it("uses the brighter themed action color on the dark Home canvas", () => {
    const action = cssRuleFor('[data-theme="dark"] .agent-home-task-message > button');

    expect(action).toContain("color: var(--brand-bright);");
  });

  it("keeps the editor width stable when a draft wraps", () => {
    const editor = cssRuleFor('.agent-workspace[data-home="true"] .agent-composer-editor-root');
    const multiline = cssRuleFor(
      '.agent-workspace[data-home="true"] .agent-composer-box[data-multiline]',
    );
    const multilineEditor = cssRuleFor(
      '.agent-workspace[data-home="true"] .agent-composer-box[data-multiline] .agent-composer-editor-root',
    );
    const multilineAttach = cssRuleFor(
      '.agent-workspace[data-home="true"] .agent-composer-box[data-multiline] .agent-composer-attach',
    );

    expect(editor).toContain("grid-column: 2;");
    expect(multiline).toContain("align-items: end;");
    expect(multilineEditor).toContain("grid-row: 1;");
    expect(multilineEditor).toContain("grid-column: 1 / -1;");
    expect(multilineAttach).toContain("grid-row: 2;");
  });

  it("aligns a wrapped Home draft with the conversation rail", () => {
    const timeline = cssRuleFor('.agent-timeline[data-home="true"]');
    const multilineEditor = cssRuleFor(
      '.agent-workspace[data-home="true"] .agent-composer-box[data-multiline] .agent-composer-editor',
    );

    expect(timeline).toContain("padding-inline: calc(var(--sp-px) + var(--sp-1) + var(--sp-2));");
    expect(multilineEditor).toContain("padding-inline: var(--sp-2);");
  });

  it("preserves whole words when markdown tables calculate column widths", () => {
    const cells = cssRuleFor(".agent-md-table th,\n.agent-md-table td");

    expect(cells).toContain("overflow-wrap: break-word;");
    expect(cells).toContain("vertical-align: top;");
  });

  it("lets the living Home mark move its body while keeping pine eyes", () => {
    const body = cssRuleFor(".clovy-alive-body");
    const sheen = cssRuleFor(".clovy-alive-sheen");
    const eyes = cssRuleFor(".clovy-alive-eye");

    expect(body).toContain("transform-box: fill-box;");
    expect(body).toContain("transform-origin: 50% 55%;");
    expect(sheen).toContain("opacity: 0.16;");
    expect(eyes).toContain("fill: var(--clovy-pine);");
  });

  it("keeps conversation and onboarding backgrounds free of the brand wash", () => {
    expect(appCss).not.toContain("--hero-wash");
    expect(tokensCss).not.toContain("--hero-wash");
    expect(appCss).not.toContain(".agent-workspace::before");
    expect(appCss).not.toContain('.agent-workspace[data-home="true"]::before');
    const veil = cssRuleFor('.agent-composer:not([data-hero="true"])::before');
    expect(veil).toContain("linear-gradient(to bottom, transparent, var(--card) 72%)");
    const topFade = cssRuleFor('.agent-workspace[data-home="true"]::after');
    expect(topFade).toContain("background: linear-gradient(to bottom, var(--card), transparent)");
    expect(topFade).toContain('content: "";');
    const personality = cssRuleFor(".onboarding-personality-stage");
    expect(personality).toContain("background: var(--card);");
    expect(personality).not.toContain("gradient");
  });

  it("drops a composer edge fade immediately when it reaches the visible caret", () => {
    const endCaret = cssRuleFor('.agent-composer-editor-root[data-caret-edge="end"]::after');

    expect(endCaret).toContain("opacity: 0;");
    expect(endCaret).toContain("transition: none;");
  });
});

describe("Clovy theme-state parity", () => {
  afterEach(() => {
    document.documentElement.removeAttribute("data-brand");
    document.documentElement.style.removeProperty("--brand");
    document.documentElement.style.removeProperty("--brand-wash");
  });

  it("lifts the semantic accent and flips its foreground in dark mode", () => {
    const darkTokens = cssRuleFor('[data-theme="dark"]', tokensCss);

    expect(darkTokens).toContain("--primary: var(--brand-bright);");
    expect(darkTokens).toContain("--primary-foreground: var(--clovy-ink);");
  });

  it("routes onboarding progress and checked switches through the semantic accent", () => {
    const progress = cssRuleFor('.onboarding-progress-seg[data-state="current"]');
    const checkedSwitch = cssRuleFor('.switch[data-state="checked"]');

    expect(progress).toContain("background: var(--primary);");
    expect(checkedSwitch).toContain("background: var(--primary);");
  });

  it("clears the active highlight from dormant send controls", () => {
    const send = cssRuleFor(".agent-composer-send:disabled");
    const steerSend = cssRuleFor(".agent-composer-steer-send:disabled");

    expect(send).toContain("box-shadow: none;");
    expect(steerSend).toContain("box-shadow: none;");
  });

  it("routes active composer sends through the selected Appearance accent", () => {
    const send = cssRuleFor(".agent-composer-send");
    const steerSend = cssRuleFor(".agent-composer-steer-send");

    for (const rule of [send, steerSend]) {
      expect(rule).toContain("background: var(--primary);");
      expect(rule).toContain("color: var(--primary-foreground);");
      expect(rule).not.toContain("var(--primary-action-background)");
    }
  });

  it("uses the active Appearance accent for home shortcut icons", () => {
    const icon = cssRuleFor(".agent-hero-chip-icon");

    expect(icon).toContain("color: var(--primary);");
  });

  it("themes the living Home character without changing its identity default", () => {
    const identityCharacter = cssRuleFor(".clovy-alive");
    const homeCharacter = cssRuleFor('.clovy-alive[data-palette="appearance"]');
    const sageCharacter = cssRuleFor(
      ':root[data-brand="sage"] .clovy-alive[data-palette="appearance"]',
    );

    expect(identityCharacter).toContain("--clovy-alive-fill-bottom: var(--clovy-glow);");
    expect(homeCharacter).toContain("--clovy-alive-fill-bottom: var(--primary);");
    expect(homeCharacter).toContain("--clovy-alive-depth-color: color-mix(");
    expect(homeCharacter).toContain("var(--primary) 68%");
    expect(homeCharacter).toContain("var(--clovy-ink)");
    expect(sageCharacter).toContain("--clovy-alive-fill-bottom: var(--clovy-glow);");
    expect(sageCharacter).toContain("--clovy-alive-sheen-color: var(--clovy-lime-top);");
  });

  it.each(BRAND_PRESETS)("exposes $id as the active Appearance preset", ({ id }) => {
    applyBrandVar(id);

    expect(document.documentElement).toHaveAttribute("data-brand", id);
  });
});
