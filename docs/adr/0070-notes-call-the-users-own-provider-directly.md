---
status: accepted
date: 2026-10-27
amends: 0065
---

# Notes call the user's own provider directly

## Context

ADR-0065 let dictation go straight from the desktop to Groq and OpenRouter
with the user's own key, and kept everything else on Clovy API (§5: "Note
transcription, note generation, chat, media, and web keep their Clovy API
routes with BYOK forwarding").

The published v0.1.2 does not ship Clovy API, and the desktop's default
`CLOVY_API_URL` is `http://127.0.0.1:8080`. On the owner's Mac and Windows
installs every meeting recording ended in a failed note reading "System:
error sending request for url (http://127.0.0.1:8080/v1/notes/transcribe) |
Microphone: ...", while dictation, already direct, worked with the same keys.

With a user key, Clovy API only relays note requests: no Clovy credits move,
and the request it sends upstream is fixed (Groq transcription parameters,
the note generation prompt, the note transcript cleanup prompt at
temperature 0). The two options were to bundle Clovy API inside the desktop
or to extend the direct route. The owner chose the direct route on
2026-10-27.

## Decision

1. **§5 of ADR-0065 now also excludes notes.** With the notes engine set to
   Clovy and the user's key saved, note transcription (final turns, the
   per-source fallback, and live preview) posts to Groq, and note generation
   and note transcript cleanup post one chat completion to OpenRouter.
   Without the key, each call goes through Clovy API exactly as before.
2. **The direct request is the one Clovy API makes.** Transcription keeps
   `response_format=json` (dictation's `verbose_json` filtering does not
   apply), the canonical `audio.wav` / `audio.m4a` file name, the dictionary
   prompt, the language, the 25 MB audio cap, and one retry of a transient
   failure. Generation and cleanup use Clovy API's own prompt files,
   compiled in from `clovy-api/crates/services/src/prompts/`.
3. **Auto stays on Clovy API.** `open-software/auto` is a selection Clovy API
   resolves, so it never takes the direct route.
4. **A request carries only the called provider's key** (ADR-0065 §2), and
   service-managed keys still live only in Clovy API (§3).
5. **Without a key and without a reachable Clovy API, the note names the
   missing key** (Groq for transcription, OpenRouter for generation) and
   where to save it, in the interface language, instead of the HTTP
   library's error. Retry stays available.
6. Clovy API's v1 routes and contracts do not change (ADR-0021).

## Consequences

- A packaged desktop with saved Groq and OpenRouter keys writes complete
  notes without running Clovy API.
- Clovy API no longer sees BYOK note requests; its logs and metrics
  undercount them, as with dictation.
- Endpoint and CLI note engines are unaffected.
- A user on Auto without Clovy API still gets a failed generation; choosing
  an explicit model is the remedy.
- Chat, media, and web keep their Clovy API routes; extending the direct
  route to them needs its own decision.

## Addendum (2026-10-27): Groq rate limits on the direct route

The direct route made the user's Groq plan the limit of note transcription.
On the base plan `whisper-large-v3-turbo` allows 20 requests a minute, 2,000
a day, 7,200 s of audio an hour and 28,800 s a day, and bills every request
as at least 10 s. v0.1.3 failed long meetings: a 429 got one retry after a
fixed 300 ms, became a failed turn, and the run kept sending the remaining
turns into the same limit, while the default-on live preview had already
spent most of the hour's audio. This addendum changes §2's "one retry of a
transient failure" for 429 and the shape of the final-transcription request.

1. **A 429 is a wait, not a failure.** The direct call reads `retry-after`
   (else `x-ratelimit-reset-requests` when no requests remain) and returns
   it as `details.retryAfterMs`, waiting it out itself only up to 2 s.
2. **One pacer per process** for Groq transcription: at most 18 requests a
   minute and 7,200 s of billed audio an hour, across both Sources, every
   note, and the live preview. A 429 holds every caller for the named wait
   (60 s when none) and repeats the same request.
3. **A long wait pauses the note.** Over 90 s, the run stops with
   `transcription_rate_limited` and `details.resumeAtMs`; the note says when
   it continues and the desktop resumes it at that time while it stays open.
   Finished turns are kept, the rest stay pending, and the full-source
   fallback does not run into the same limit.
4. **With the user's key, speech turns go in packs.** Consecutive turns of
   one Source are concatenated (0.5 s apart) into one request of up to
   600 s, the chunk of Groq's own long-audio example (19.2 MB of 16 kHz mono
   PCM16, under the 25 MB cap), sent as `verbose_json` with segment times
   and split back per turn. Silence between turns is never sent, the
   10-second minimum is paid once per pack, and each turn keeps its own
   durable job and Source attribution. Clovy API's route has no segment
   times, so without the key a turn is still one request. Fixed splits (a
   full-Source job, an over-long turn) use 600 s chunks with a 10 s overlap
   whose repeated words are dropped. FLAC was not adopted: Groq bills
   seconds, not bytes, and the project has no encoder.
5. **The live preview gives way.** It never waits for the pacer and spends at
   most 3,600 s of the hour's audio; past that, preview chunks are dropped
   (ADR-0002 allows it) so the final transcription keeps its budget. The
   preview's text is not reused for the note: its fixed 8 s cuts and
   missing dictionary prompt are worse than the turn-based pass.

The limits are the base plan's; a paid plan is paced the same until Clovy
can read the plan. Clovy API does not change.

Review of the same addendum (2026-10-27, before release):

6. **Split jobs keep their chunks.** A full-Source job (microphone only,
   the full-Source fallback) or an over-long turn keeps each finished 600 s
   chunk in `note_transcription_chunks`, so a resume sends only the chunks
   still missing; without it, two audible hours paused before the last chunk
   on every run and never finished.
2. (amended) **One pacer per Groq account, on the wall clock.** The pacer is
   picked per request: the saved key's account, by a SHA-256 fingerprint
   that never keeps the key, or Clovy API's route without a key, which has
   its own. Its pause and its hour of audio are wall-clock times, guarded
   against a clock set back, so a pause ends on time across sleep.
3. (amended) **A resume claims its pause once.** The automatic resume sets
   the note from that very pause to transcribing in one update; a Retry
   that got there first leaves nothing to claim, and a retry that waited
   behind another run skips a note that no longer needs it.

Second review of the same addendum (2026-10-28, before release):

- **The model is part of what a resume reuses.** A job's durable identity
  and each kept chunk's key include the transcription model a run reads
  once (and the chunk's language); the default model adds nothing to the
  identity, so sessions transcribed before keep theirs.
- **One route per request.** A request reads the saved key once and its
  kept chunks, pacer and transport use that same key and model.
- **Every retry revalidates under the lock.** The note's queue counts
  finished runs; a retry that sees a run finish after it was queued, or a
  ready note, does not process the note again, whatever its queue depth.

Third review of the same addendum (2026-10-28, before release):

- **An unreadable saved key is an error, not Clovy API's route.** Only no
  saved key takes Clovy API's route; a saved key the credential store
  cannot return fails the request with `provider_api_key_unavailable`,
  and the live preview sends nothing.
- **A manual retry marks only a still-failed note.** Its early
  transcribing status is a compare-and-set from failed; a retry that made
  it always runs, so a skipped run never leaves the note in between.


## Addendum - 2026-10-28: direct providers only

The missing-key server fallback, server prompt source and Auto exception are superseded for this fork by
[ADR-0073](0073-desktop-calls-user-providers-without-a-clovy-server.md). The desktop calls the user's providers directly; chat stays
on CLI engines and Clovy tools remain local through MCP. The server is removed,
not replaced or deployed with a breaking contract. Earlier decisions above
remain as historical records; current behavior follows ADR-0073 and ADR-0072.
