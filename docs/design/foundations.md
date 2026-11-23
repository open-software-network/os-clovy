# Design foundations

The theming model, design tokens, and type system that everything in Clovy's UI
builds on. Companion docs: [components.md](components.md) (the shared UI
primitives), [conventions.md](conventions.md) (naming and interaction rules),
and [taste.md](taste.md) (the sensibility behind the rules).

Live values render in the styleguide: run `pnpm dev`, then open
`http://localhost:1421/styleguide.html` (foundations sections: `color`, `type`,
`spacing`, `radius`, `elevation`, `motion`, `controls`; component and pattern
sections are listed in [components.md](components.md)). Prefer reading a
token's live swatch there over eyeballing a hex.

## Theming

The palette is driven by a single accent, `--brand`, plus its chroma-capped
companion `--brand-wash`. Both are `@property`-registered colors, so a theme
change animates instead of snapping. Surface neutrals mix a sliver of
`--brand-wash` (the accent with its chroma capped) so a high-chroma accent tints
the greys the same amount a dusty one does; foreground/text tokens stay pure
neutral for legibility.

Five presets ship (rose, clay, sage, ocean, plum), sage is the default; a preset
applies by setting the `--brand` / `--brand-wash` variables (see
`src/lib/brand.ts`). Derived tokens (`--brand-tint`, `--brand-line`, `--warm-*`,
and surface neutrals) re-mix from `--brand` automatically, so recoloring the
accent recolors the supporting UI. Clovy identity tokens and identity-led solid
actions stay fixed across presets; Appearance-led Home details and composer
send affordances remain preset-driven. Sage is the character's native
appearance, so its Home and themed dock variants preserve canonical lime while
the other presets recolor the character itself. Light and dark are separate
concerns: dark mode is a `data-theme="dark"` cascade that overrides colors and
shadows (light-tuned black shadows vanish on near-black). See
[conventions.md](conventions.md) for the brand pipeline and the `index.html`
sync warning.

### Conversation backgrounds

New session and Home use a single neutral Drift field behind the HTML content.
Appearance's Background preference offers Drift (default), Image, and None, stored as
`os-clovy:background` and applied live. The field uses `--agent-background-ink`
(`#8f8d86` in both themes), independent of the accent, at a barely visible 0.14 opacity
from the single `DRIFT_OPACITY` constant. It covers the entire surface, including behind the heading, composer, and
suggestions, with moving organic voids from flow contrast 1.4 and scale 1.6 and no cleared column. Its host extends across the panel body padding to
reach every panel edge, clipped by the panel's existing rounded corners. The existing composer card protects its
controls; the field never intercepts pointer events. The kit pauses it offscreen or in a hidden window and holds
a still frame for reduced motion; missing WebGL2 or a failed load leaves the
plain surface. Notes, settings, and HUDs do not mount it. The onboarding
personality card uses the plain card surface. Conversation scroll fades remain; Home confines the composer dissolve to its card
so it cannot erase the field along the panel bottom.

Image uses a local PNG, JPEG, or WebP copy up to 20 MiB, with no upload. It covers
the panel, centered, under a single 0.82 veil in the theme's `--card` color.
Appearance lets the user choose, replace, or remove it. Missing images leave the
conversation surface plain and are reported only in settings. Drift, Image,
and None are mutually exclusive.

Without a saved theme the app opens dark; saved light, dark, or system choices
win. The pre-paint scripts in `index.html`, `styleguide.html`, and
`onboarding-preview.html` must stay in sync with `src/lib/theme.ts`.

## Tokens

All tokens live in `src/styles/tokens.css`. Reach for a token before hand-coding
a value (see [spec/design-tokens](../../spec/design-tokens.md)).

- **Type** (`--font-*`, `--fs-*`, `--fw-medium`): the type system, below.
- **Spacing** (`--sp-px` to `--sp-12`, 2px to 40px): all gaps, padding, and
  margins. Layout widths (`--content-max`, `--chat-max`, sidebar dimensions)
  are their own named tokens.
- **Radius** (`--r-xs` to `--r-2xl`, `--r-pill`, `--r-window`): corner rounding;
  `--r-window` matches the native macOS window curve for inset surfaces.
- **Shadow** (`--shadow-sm/md/lg`, `--shadow-inset`): elevation. Shadows never
  carry rings; call sites compose a border or `--shadow-inset` separately (see
  [conventions.md](conventions.md)).
- **Motion** (`--ease-*`, `--t-fast/med/slow`): easing curves and durations.
- **Control sizes** (`--control-xs` to `--control-xl`, 22px to 36px):
  interactive control heights. Do not hand-roll min/max heights on buttons,
  inputs, or selects (see [spec/control-sizes](../../spec/control-sizes.md)).
- **Appearance accent** (`--primary`, `--primary-foreground`, `--brand-bright`):
  preset-driven color for selection, progress, checked controls, composer send
  affordances, Home shortcuts, and the living Home character outside Sage.
  Light uses the active brand; dark uses its luminous companion with dark ink
  so the same role stays visible without changing hue families.
- **Primary actions** (`--primary-action-background*`,
  `--primary-action-border`, `--primary-action-foreground`): a separate,
  theme-adaptive Clovy identity treatment - deep pine with a restrained lime
  edge on light surfaces, and a lime field with dark Clovy ink on dark
  surfaces. Use these tokens for identity-led actions such as onboarding and
  setup, not preset-linked composer controls.

## Type system

### Scale

Font sizes come only from the `--fs-*` scale (see
[spec/type-scale](../../spec/type-scale.md)):

| Token | Size | Role |
|---|---|---|
| `--fs-2xs` | 10px | Micro labels, dense badges |
| `--fs-xs` | 11px | Labels, badges, metadata |
| `--fs-sm` | 12px | Descriptions, support copy |
| `--fs-md` | 13px | Body (the root default) |
| `--fs-lg` | 14px | Row titles, markdown headings |
| `--fs-xl` | 16px | View and dialog titles |
| `--fs-2xl` | 20px | Larger headings |
| `--fs-display` | 30px | Display and marketing moments |

### Heading mapping

The as-rendered hierarchy. "Renders 400" means the underlying face is Regular
even where an older declaration says 500, because the family has only two faces
(below).

| Element | Class | Size | Weight |
|---|---|---|---|
| View / page title | `.agent-rail-header h1`, `.agent-detail-header h2` | `--fs-xl` | `--fw-medium` |
| Dialog title | `.dialog-title` | `--fs-xl` | 400 |
| Empty-state title | `.empty-state-title` (serif) | `--fs-xl` | 400 |
| Settings group heading | `.settings-group-heading` (h2, muted) | `--fs-md` | 400 |
| Settings row title | `.settings-row-title` (h3) | `--fs-lg` | 400 |
| Markdown heading | `.agent-markdown h2` / `h3` | `--fs-lg` | `--fw-medium` |
| Body | root default | `--fs-md` | 400 |
| Description / support | in `--muted-foreground` | `--fs-md` or `--fs-sm` | 400 |
| Micro label / badge | | `--fs-xs` or `--fs-2xs` | 400 |
| Display / marketing | welcome, sign-in | `--fs-display` | 400 |

### Two-weight system

ABC Diatype ships **only** Regular (400) and Medium (600) faces with
`font-synthesis: none`. A `font-weight: 500` silently renders 400 and `700`
renders 600, so those declarations lie. Only two values are allowed: the default
400, which is the app's voice, and `var(--fw-medium)` (600), used sparingly for
headings, row titles, and structural emphasis. Never write raw 500 / 600 / 700 /
bold (see [spec/font-weights](../../spec/font-weights.md)).

### Family roles

- **Sans** `--font-sans` (ABC Diatype) is the primary voice, for nearly all UI.
- **Serif** `--font-serif` (Martina Plantijn, faces 300 to 400) is limited to
  headings and display moments, e.g. empty-state titles.
- **Mono** `--font-mono` (Berkeley Mono, 400 only) only where it earns it: code
  and technical identifiers.

See [spec/font-families](../../spec/font-families.md).

### No all caps

No ALL CAPS anywhere, especially eyebrows, pre-headers, and metadata. There is
no `text-transform: uppercase` in the app today and the rule keeps it that way
(see [spec/no-all-caps](../../spec/no-all-caps.md) and
[spec/sentence-case](../../spec/sentence-case.md)).

### No tabular numerals

UI numbers use the typeface's proportional figures. See
[spec/no-tabular-numerals](../../spec/no-tabular-numerals.md) for the rule and
its narrow live-ticking exception.

## Known deviations

Pass-2 worklist seeds. These are documented, not yet resolved, and Andrew
decides the target in-browser:

- **Note-preview headings** (resolved 2026-07-06): the raw 15px h1/h2 moved
  to `--fs-lg`, matching agent markdown headings. Still open: the one
  remaining 15px in app.css is the `+`/`-` tool-disclosure glyph (around
  2225), a decorative mono glyph that needs a size decision, not a heading.
- **Raw control heights**: scattered raw min/max-heights on controls that should
  move to the `--control-*` tokens.
- **Toggle wrapper names** (resolved 2026-07-06): an audit found every real
  on/off switch already uses the `Switch` component; classes like
  `.mcp-server-toggle` are layout wrappers or disclosure buttons, not bespoke
  toggles. Optional: rename the two wrapper classes to `-toggle-wrap` so the
  names stop implying otherwise (deferred behind in-flight settings work).
- **Bespoke input classes** (`dialog-input`, `settings-secret-input`,
  `mcp-add-input`, ...) pending a shared field treatment.
- **Hand-rolled menus and popovers** (sidebar identity menu, context menus,
  composer `@` / slash menus) pending a shared positioning helper.
- **Solid-action foregrounds** (resolved 2026-08-05): text/icons on fixed dark
  and destructive solids use `--on-solid`; theme-adaptive Clovy primary actions
  use `--primary-action-foreground`. Still open: the sidebar recording
  indicator's bespoke always-dark pill uses raw oklch values and needs a
  dark-surface token family decision.
