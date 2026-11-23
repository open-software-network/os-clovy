---
status: accepted
date: 2026-10-15
supersedes: ADR-0056 decision 4 (formatting)
---

# Dates, times, and numbers follow the interface language, and native errors are worded by code

## Context

[ADR-0056](0056-interface-language-uses-an-in-repo-typed-catalog.md) decision 4
kept `undefined` as the `Intl` locale for English so English users would see
their system's regional format unchanged. In practice the English copy around
those values is fixed English, while the values took the system's words: an
English interface on a Portuguese macOS showed "Segunda-feira at 8:00" on the
Home thread, and the activity timeline defaulted some call sites to English
even when the interface was Portuguese. The owner decided that switching the
interface language must switch every visible date, time, and named value.

Native error text has the same problem from the other side. The dictation HUD,
Settings, the Home conversation, and the note failure banner showed English
sentences written by Rust or the Swift dictation helper whatever the interface
language was.

## Decision

1. **The `Intl` locale is always the interface language.** `intlLocale()`
   returns `"en"` or `"pt-BR"`, never `undefined`, so weekday and month names,
   AM/PM, the "at"/"às" connector, and separators always match the words
   around them. No surface formats with the system locale (`toLocale*([])`,
   a hard-coded `"en-US"`, or a default parameter of `"en"`).
2. **The date-format preference picks only the order.** `clovy:date-format`
   (`system` / `month-first` / `day-first`) chooses day/month order; `system`
   takes that order from the system's regional format. Names always come
   from the interface language. When the system order matches the language's
   own pattern, the language's pattern is used as is ("15 de out.").
3. **English keeps its fixed shapes.** Where English copy always used raw
   digits, `toFixed` decimals, or `$1.75`, it keeps them; code tests
   `isEnglishLocale()` instead of `intlLocale() === undefined`.
4. **Native errors are worded by stable code.** Rust and Swift keep writing
   English messages and never translate. Every user-facing native error
   carries a stable `code`; `src/lib/errors.ts` maps known codes to catalog
   keys (`messageForErrorCode`, also used by `messageFromError`), and the
   native message is only the fallback for codes without an entry. Note
   failures persist only their English sentence, so the note banner matches
   those exact sentences from `domain/processing.rs` against the English
   catalog and shows the interface-language copy.

## Consequences

- An English interface on a non-English system now formats numbers as
  `1,234.5` and dates as English words; this is the intended change.
- A new native error needs a stable code and, to be translated, an entry in
  `ERROR_CODE_MESSAGES` with catalog keys whose English text matches the
  native message. Unmapped codes still show the native English message.
- Changing a note failure sentence in `processing.rs` requires the same change
  to its `notes.failure.*` English entry; the Rust test
  `stored_failure_sentences_match_the_desktop_catalog` fails until both match.
  Persisting the code instead needs a new `notes` column (schema migration),
  so it was deferred.
- Stored routine intervals ("every 30m") stay English on disk (a wire value)
  and are phrased from the catalog for display.
- Rejected alternatives: keeping the system locale for English (the reported
  bug), translating inside Rust/Swift (a second catalog outside the typed one,
  and the native side does not know the interface language at every emit
  site), and persisting an error code on notes (a storage change for a banner
  that can match its own backend's fixed sentences).
