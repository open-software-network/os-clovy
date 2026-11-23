---
status: accepted
date: 2026-09-26
---

# Interface language uses an in-repo typed catalog, mirrored to native menus by event

## Context

Clovy shipped English-only UI copy hard-coded across React components, the
three vanilla-TS HUD windows, `src/lib` helpers, the Rust app and menu bar
menus, and the browser extension popup. The owner asked for a Brazilian
Portuguese interface with a visible language choice. The existing
dictation/transcription language setting is a speech hint and says nothing
about the UI language.

Constraints that shaped the design:

- Supply-chain policy (`spec/package-install-security.md`) makes every new
  dependency a reviewed, cooled-down addition, and the frontend has several
  separate entry points (main window, `hud.html`, `agent-hud.html`,
  `meeting-hud.html`) that must share one choice.
- Preferences are already per-machine localStorage values in the shared webview
  origin (theme, brand, text size), bridged to June-era keys by
  `storage-compat`.
- Native menus are built in Rust before any webview script runs.
- Existing installs must not change language under the user on upgrade.

## Decision

1. **No i18n library.** `src/i18n/` holds a small typed catalog:
   `defineMessages({ en, "pt-BR" })` per product-area namespace, `t(key,
   params)` with `{name}` interpolation, plural objects selected by
   `Intl.PluralRules` (Portuguese zero reads as plural), `translateRich` for
   inline markup, and `Intl`-based date/number/list formatters. English is the
   source of truth; the Portuguese catalog must have the same keys and plural
   shapes, enforced by TypeScript and by `src/test/i18n-core.test.tsx`.
2. **Preference.** `os-clovy:interface-locale` in localStorage (dual-written to
   the June-era alias by the existing bridge). Unknown or malformed values fall
   back to English and are repaired on startup. When nothing is stored, an
   install that has already completed onboarding keeps English; a fresh install
   follows a supported system language. The resolved value is written back so it
   stays stable after onboarding completes.
3. **Propagation.** The main window emits the Tauri event
   `clovy://interface-locale` on startup and on change. HUD windows follow it
   (plus the `storage` event); Rust mirrors it into `interface_locale.rs` and
   rebuilds the app menu and the menu bar tray. English menu text is
   byte-identical to the pre-i18n literals and AppKit predefined items keep the
   platform default in English.
4. **Formatting.** English keeps `undefined` as the `Intl` locale (the system's
   regional format, unchanged behavior); Portuguese formats as `pt-BR`.
   Currency amounts keep their currency (USD shows as `US$`).
5. **Out of scope by design.** User content, model output, text sent to the
   model as instructions, and wire/storage identifiers are never translated.
   The browser extension cannot read the app preference, so it follows the
   browser UI language.

## Consequences

- Adding a locale is a new catalog column plus an entry in
  `SUPPORTED_LOCALES`; the compiler lists every missing key.
- Components must call `useT()` so they re-render on a language change, and
  non-React code must call `t()` at use time, never at module load. Exported
  option tables use getters to keep their shape.
- Rust menus show English until the webview has started and emitted the
  locale (a fraction of a second at launch).
- Strings persisted at creation time (for example canned Home replies or a
  stored session title) stay in the language that was active then.
- Rejected alternatives: i18next/FormatJS (new dependency surface and runtime
  for a two-locale catalog), remounting the React tree on change (loses open
  UI state such as the settings page being used to switch), and keeping the
  preference in Rust (the native side would need a new command surface while
  every other appearance preference already lives in webview storage).

## Addendum (2026-10-15)

Decision 4 (formatting) is superseded by
[ADR-0061](0061-dates-and-numbers-follow-the-interface-language.md): `Intl`
always formats in the interface language, never the system locale.
