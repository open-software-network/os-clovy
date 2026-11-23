import { describe, expect, it } from "vitest";
import appCss from "../styles/app.css?raw";

function cssRuleFor(selector: string) {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = new RegExp(`(?:^|\\n)${escaped}\\s*\\{`).exec(appCss);
  if (!match) throw new Error(`Missing CSS rule for ${selector}`);
  const openIndex = match.index + match[0].length - 1;
  let depth = 0;
  for (let index = openIndex; index < appCss.length; index += 1) {
    if (appCss[index] === "{") depth += 1;
    if (appCss[index] === "}") {
      depth -= 1;
      if (depth === 0) return appCss.slice(openIndex + 1, index);
    }
  }
  throw new Error(`Unclosed CSS rule for ${selector}`);
}

describe("pending action card styles", () => {
  it("keeps long links inside approval cards", () => {
    expect(cssRuleFor(".agent-approval-card p")).toContain("overflow-wrap: anywhere;");
    expect(cssRuleFor(".agent-approval-card pre")).toContain("max-width: 100%;");
    expect(cssRuleFor(".agent-approval-card pre")).toContain("overflow-wrap: anywhere;");
  });

  it("sets card body/description prose one step up at --fs-md (shared across all card types)", () => {
    // The shared body-prose rule that every card type's <p> flows through — bumped
    // from --fs-sm to --fs-md so the description reads at the body tier. The
    // pre/mono block stays at the smaller --fs-xs.
    const body = cssRuleFor(".agent-approval-card p,\n.agent-clarify-card p");
    expect(body).toContain("font-size: var(--fs-md);");
    expect(cssRuleFor(".agent-approval-card pre")).toContain("font-size: var(--fs-xs);");
  });

  it("gives the secret label the system field-label treatment", () => {
    // Matches .dialog-field-label (the canonical partner of the .dialog-input the
    // field already uses): --fs-md, full-strength foreground, weight 400 — no
    // longer the quieter --fs-xs muted eyebrow. The note stays quieter one step
    // down at --fs-sm (bumped up from --fs-xs, which read tiny once the body
    // prose moved to --fs-md).
    const label = cssRuleFor(".agent-secret-label");
    expect(label).toContain("font-size: var(--fs-md);");
    expect(label).toContain("color: var(--foreground);");
    expect(label).toContain("font-weight: 400;");
    const note = cssRuleFor(".agent-secret-note");
    expect(note).toContain("font-size: var(--fs-sm) !important");
    expect(note).toContain("color: var(--muted-foreground)");
  });

  it("is single-column with no icon tile and no inline title glyph", () => {
    const card = cssRuleFor(".agent-approval-card,\n.agent-clarify-card");
    // The card is a plain block, not the old two-column grid.
    expect(card).toContain("display: block;");
    expect(card).not.toContain("grid-template-columns");
    // The round-1 inline title glyph is gone entirely — no CSS rule for it.
    expect(() =>
      cssRuleFor(
        ".agent-approval-card > .agent-tool-icon,\n.agent-clarify-card > .agent-tool-icon",
      ),
    ).toThrow();
    // Titles across all five card types land on the shared row-title scale
    // (including the condensed approval/sudo header title).
    const title = cssRuleFor(
      ".agent-approval-card .agent-tool-title > span:first-child,\n.agent-clarify-card .agent-tool-title > span:first-child,\n.agent-action-card-title",
    );
    expect(title).toContain("font-size: var(--fs-lg);");
    expect(title).toContain("font-weight: var(--fw-medium);");
  });

  it("drops the shadow on the approval and clarify cards but keeps it on the other surfaces", () => {
    // The approval/clarify cards go shadowless in the quieter redesign.
    expect(cssRuleFor(".agent-approval-card,\n.agent-clarify-card")).not.toContain("box-shadow");
    // The remaining system surfaces keep their soft elevation.
    expect(cssRuleFor(".agent-safety-panel,\n.agent-tool-event")).toContain(
      "box-shadow: var(--shadow-sm);",
    );
  });

  it("clamps the collapsed description and keeps the Details disclosure chrome (sudo mode notice)", () => {
    // The collapsed card shows the prose description clamped to two lines...
    const description = cssRuleFor(".agent-action-card-description[data-clamped]");
    expect(description).toContain("display: -webkit-box;");
    expect(description).toContain("-webkit-line-clamp: 2;");
    // ...the old truncated mono summary line is gone entirely (no CSS rule for it).
    expect(() => cssRuleFor(".agent-action-card-summary-line")).toThrow();
    expect(() => cssRuleFor(".agent-action-card-summary")).toThrow();
    expect(() => cssRuleFor(".agent-action-card-command")).toThrow();
    // The unrestricted mode survives collapsed as a small caution badge (the
    // blast radius must read before expanding). It is a quiet neutral-gray pill
    // — the leading caution glyph + "Unrestricted" label carry the meaning
    // without a pop of color (neither the error-reserved destructive red nor a
    // brand/warm accent).
    const modeBadge = cssRuleFor(".agent-sudo-mode-badge");
    expect(modeBadge).toContain("flex: 0 0 auto;");
    expect(modeBadge).toContain("background: var(--surface-subtle);");
    expect(modeBadge).toContain("color: var(--muted-foreground);");
    expect(modeBadge).toContain("font-weight: 400;");
    expect(modeBadge).not.toContain("var(--destructive");
    expect(modeBadge).not.toContain("var(--warm");
    // ...and the old bare-text lowercase tag is gone entirely.
    expect(() => cssRuleFor(".agent-sudo-mode-tag")).toThrow();
    // ...and the Details disclosure chevron rotates when the card is expanded.
    expect(
      cssRuleFor('.agent-action-card-details[aria-expanded="true"] .agent-disclosure-chevron'),
    ).toContain("transform: rotate(180deg);");
  });

  it("frames the split button as an outlined container with gray-filled segments", () => {
    // The container is just a hairline outline (transparent bg) binding the two
    // segments; the gray fill lives on the buttons themselves. Approve reads as
    // primary by being filled, against the ghost Deny — no dark or
    // colored weight. The risk lives in the request, not the button.
    const split = cssRuleFor(".agent-approval-split");
    expect(split).toContain("background: transparent;");
    expect(split).toContain("border: 1px solid var(--border-subtle);");
    expect(split).toContain("border-radius: var(--r-md);");
    expect(split).toContain("padding: 2px;");
    expect(split).toContain("gap: 2px;");
    // Disabled dims the whole control.
    expect(cssRuleFor(".agent-approval-split-disabled")).toContain("opacity: 0.55;");
    // Approve carries the solid gray fill (--surface-subtle), firming to --muted
    // on hover — no green, no dark primary fill.
    const approve = cssRuleFor(".agent-approval-approve");
    expect(approve).toContain("border-radius: calc(var(--r-md) - 3px);");
    expect(approve).toContain("background: var(--surface-subtle);");
    expect(approve).toContain("color: var(--secondary-foreground);");
    expect(approve).not.toContain("var(--success)");
    expect(cssRuleFor(".agent-approval-approve:hover:not(:disabled)")).toContain(
      "background: var(--muted);",
    );
    // The caret shares Approve's gray fill so the split reads as one control,
    // divided only by the 2px gap (no seam/border).
    const scope = cssRuleFor(".agent-approval-scope");
    expect(scope).not.toContain("border-left");
    expect(scope).toContain("border-radius: calc(var(--r-md) - 3px);");
    expect(scope).toContain("background: var(--surface-subtle);");
    expect(cssRuleFor(".agent-approval-scope:hover:not(:disabled)")).toContain(
      "background: var(--muted);",
    );
  });

  it("styles scope-menu item focus exactly like hover, with no default ring", () => {
    // Focus shares the hover background...
    const focusHover = cssRuleFor(
      ".agent-approval-scope-item:hover:not(:disabled),\n.agent-approval-scope-item:focus:not(:disabled)",
    );
    expect(focusHover).toContain("background: var(--surface-subtle);");
    // ...and the default outline is dropped (focus stays visible via the background).
    expect(cssRuleFor(".agent-approval-scope-item:focus")).toContain("outline: none;");
    // With the mouse in the menu, a focused-but-not-hovered item drops its
    // background so hover and focus never light two rows at once.
    expect(
      cssRuleFor(".agent-approval-scope-menu:hover .agent-approval-scope-item:focus:not(:hover)"),
    ).toContain("background: transparent;");
  });

  it("gives clarify choices a light fill and no border (no border-within-border)", () => {
    // The card already carries a border, so the choice rows drop theirs and read
    // as a light neutral fill instead, firming one step on hover.
    const choice = cssRuleFor(".agent-clarify-choices button");
    expect(choice).toContain("border: 0;");
    expect(choice).not.toContain("border: 1px solid var(--border-subtle);");
    expect(choice).toContain("background: var(--surface-subtle);");
    expect(cssRuleFor(".agent-clarify-choices button:hover:not(:disabled)")).toContain(
      "background: var(--muted);",
    );
    // A little extra air separates the textarea from the actions row.
    expect(cssRuleFor(".agent-clarify-form div")).toContain("margin-top: var(--sp-1);");
  });

  it("keeps Deny as a plain neutral ghost button (only Approve is colored)", () => {
    // Footer ghost buttons share a soft neutral gray hover (no exclusion
    // for Deny, which is no longer red).
    const ghostHover = cssRuleFor(".agent-approval-actions .btn-ghost:hover:not(:disabled)");
    expect(ghostHover).toContain("background: var(--surface-subtle);");
    // Deny wears no alarm color: its old destructive-soft hover rule is gone
    // (denying is the safe choice; red is reserved for destructive states).
    expect(() => cssRuleFor(".agent-approval-deny:hover:not(:disabled)")).toThrow();
    // Deny stays quiet at rest (muted, the secondary choice).
    expect(cssRuleFor(".agent-approval-deny")).toContain("color: var(--muted-foreground);");
  });

  it("drops the redundant header status rule, leaving the tool-row pill intact", () => {
    // The pending-card "Waiting" status is gone (redundant with the title and
    // the visible action buttons), so its header-scoped override no longer
    // exists.
    expect(() =>
      cssRuleFor(".agent-action-card-header .agent-tool-live-status[data-status]"),
    ).toThrow();
    // The shared base .agent-tool-live-status (tool-disclosure running/failed
    // rows) is untouched — it keeps its pill fill.
    expect(cssRuleFor(".agent-tool-live-status")).toContain("background: var(--surface-subtle);");
  });

  it("sizes the secret input proportionate to the compact footer buttons", () => {
    // Scoped under .agent-secret-form (0,2,0) so it outranks the later
    // .dialog-input base (0,1,0): shorter --control-md height, tighter --sp-2
    // vertical padding, and --fs-sm mono so the masked dots don't read chunky.
    const secret = cssRuleFor(".agent-secret-form .agent-secret-input");
    expect(secret).toContain("min-height: var(--control-md);");
    expect(secret).toContain("padding: var(--sp-2) var(--sp-3);");
    expect(secret).toContain("font-family: var(--font-mono);");
    expect(secret).toContain("font-size: var(--fs-sm);");
    // The dialog-input base still owns the larger default the scope overrides.
    expect(cssRuleFor(".dialog-input,\n.dialog-textarea")).toContain(
      "min-height: var(--control-lg);",
    );
  });

  it("reserves timestamp clearance below a card-bearing turn only", () => {
    // A turn that holds an action card gets padding-bottom so the absolutely
    // positioned timestamp row (anchored at the turn body floor) clears the
    // card's bottom border. React marks only card-bearing turns, so text turns
    // remain untouched without an ancestor-sensitive selector.
    const cleared = cssRuleFor(".agent-assistant-turn-body-action-card");
    expect(cleared).toContain("padding-bottom: var(--sp-2);");
    // The base turn body carries no such padding (text turns stay tight).
    expect(cssRuleFor(".agent-turn-actions-inner")).toContain("padding-top: var(--sp-px);");
  });
});

describe("resolved action row styles", () => {
  it("tightens consecutive activity rows without tightening the answer gap", () => {
    const activitySelector = `.agent-assistant-turn-body > :is(
    .agent-reasoning,
    .agent-tool-stack,
    .agent-resolved-row,
    .agent-approval-card[data-status="pending"],
    .agent-clarify-card[data-status="pending"],
    .agent-action-card[data-status="pending"],
    .agent-system-notice[data-status="pending"]
  )
  + :is(
    .agent-reasoning,
    .agent-tool-stack,
    .agent-resolved-row,
    .agent-approval-card[data-status="pending"],
    .agent-clarify-card[data-status="pending"],
    .agent-action-card[data-status="pending"],
    .agent-system-notice[data-status="pending"]
  )`;
    const compactActivity = cssRuleFor(activitySelector);
    // The turn's base gap is --sp-4 (10px). Only an adjacent tool/receipt row
    // pulls back by --sp-2 (6px), yielding --sp-1 (4px).
    // Text is absent from both selector lists, so the response keeps 10px.
    expect(cssRuleFor(".agent-assistant-turn-body")).toContain("gap: var(--sp-4);");
    expect(cssRuleFor(".agent-tool-stack")).toContain("gap: var(--sp-1);");
    expect(compactActivity).toContain("margin-top: calc(-1 * var(--sp-2));");
    expect(activitySelector).toContain('.agent-approval-card[data-status="pending"]');
    expect(activitySelector).toContain('.agent-clarify-card[data-status="pending"]');
    expect(activitySelector).toContain('.agent-action-card[data-status="pending"]');
    expect(activitySelector).toContain('.agent-system-notice[data-status="pending"]');
  });

  it("reuses the tool-disclosure row treatment so the receipt matches the tool rows", () => {
    // The receipt carries .agent-tool-disclosure (verified in the DOM tests), so
    // its sizing/hover/icon-swap come from that shared block — no duplicated
    // numbers here. This modifier only owns the receipt-specific bits.
    // The expanded body is hidden while collapsed — the row is one line.
    expect(cssRuleFor(".agent-resolved-row:not([open]) > .agent-resolved-body")).toContain(
      "display: none;",
    );
    // The detail truncates to a single line with an ellipsis.
    const detail = cssRuleFor(".agent-resolved-detail");
    expect(detail).toContain("text-overflow: ellipsis;");
    expect(detail).toContain("white-space: nowrap;");
    // The body indents under the 15px tool-row icon cell.
    expect(cssRuleFor(".agent-resolved-body")).toContain("calc(15px + var(--sp-2))");
  });

  it("keeps a denied outcome quiet (no destructive tint) — read via glyph and label", () => {
    // Denying is a safe choice, not a destructive one, so the receipt carries no
    // red: the deny-tint rules for the glyph and label are gone entirely.
    expect(() =>
      cssRuleFor('.agent-resolved-row[data-choice="deny"] .agent-resolved-icon-glyph'),
    ).toThrow();
    expect(() =>
      cssRuleFor('.agent-resolved-row[data-choice="deny"] .agent-resolved-label'),
    ).toThrow();
  });
});
