# Scribe Audio Recording Consent — Launch Checklist Spec

**Created**: 2026-06-12
**Status**: Draft
**Source**: OpenSoftware Launch Checklist (Google Doc)

---

## Overview

Scribe records audio from meetings and voice notes. When recording meetings — especially those involving multiple participants — consent is a legal and ethical requirement. This spec defines four consent mechanisms that Scribe must implement before public launch, drawn from the OpenSoftware Launch Checklist. The requirements draw on recognized industry safe-harbor practices for recording consent.

## Requirements

### REQ-1: Audible recording-start beep

When Scribe begins recording a meeting, it must emit an audible beep that is captured in the recording itself and audible to all participants.

**Acceptance Criteria**:

1. An audible tone plays through the system audio output at the start of every meeting recording.
2. The tone is also captured in the recorded audio stream, so it is present in the resulting transcript and audio file.
3. The tone is distinct from notification sounds and is clearly recognizable as a recording indicator.
4. The beep occurs before any meeting audio is captured (i.e., the first audio frame in the recording is the beep).
5. The beep is not skippable or configurable by the user — it always plays.
6. The beep duration and frequency are defined in a shared constant (not hardcoded per platform).

**Rationale**: An audible beep at the start of a recording is a recognized industry safe-harbor for recording consent. It provides constructive notice to all participants, including those who may not see a screen or visual indicator.

### REQ-2: Identify the transcription bot

When Scribe joins or records a meeting, it must clearly identify itself as a transcription bot to all participants.

**Acceptance Criteria**:

1. When joining a meeting platform (Zoom, Google Meet, etc.), the bot's display name clearly indicates it is a transcription bot (e.g., "June (Recording)" or "June Notetaker").
2. If the meeting platform supports a join message or bot profile, the description states that the bot is recording and transcribing the meeting.
3. For in-person / system-audio recordings (no bot join), the audible beep (REQ-1) and visible indicator (REQ-3) serve as the identification mechanism.

**Rationale**: Participants must know that a recording and transcription is taking place and who is performing it. Opaque or ambiguous bot names undermine informed consent.

### REQ-3: Visible recording indicator throughout the session

A persistent, visible indicator must be displayed for the entire duration of a meeting recording.

**Acceptance Criteria**:

1. A recording indicator is visible on screen whenever Scribe is actively recording a meeting.
2. The indicator remains visible for the entire duration of the recording — it does not auto-hide or dismiss after a timeout.
3. The indicator clearly communicates that recording is in progress (e.g., a red dot with "Recording" label, a pill in the menu bar, or an overlay).
4. When recording stops, the indicator is removed.
5. The indicator is visible regardless of which application has focus — it is not hidden behind other windows.
6. The indicator is distinguishable from other status indicators in the June UI.

**Rationale**: A persistent, always-visible indicator ensures that all participants in the same physical space can see that recording is active, even if they missed the initial beep. This is a standard practice in recording applications (Zoom, Otter.ai, etc.).

### REQ-4: One-click consent language sharing

Scribe must provide a one-click feature to send consent language to all meeting participants ahead of the meeting.

**Acceptance Criteria**:

1. When a user schedules or initiates a meeting recording, Scribe offers a one-click action to share consent language with participants.
2. The consent language clearly states that the meeting will be recorded and transcribed by June, who owns the recording, and how it will be used.
3. The sharing mechanism integrates with the user's calendar (e.g., adds a note to the calendar invite) or communication tool (e.g., sends a message via email or Slack).
4. The consent language template is editable by the user in Settings.
5. A default consent language template is provided out of the box.
6. The one-click action is surfaced at the point of scheduling or starting a recording, not buried in settings.

**Rationale**: Proactively sharing consent language before a meeting is a best practice that goes beyond the minimum legal requirement in many jurisdictions. It demonstrates good faith and reduces the risk that participants are unaware of the recording.

## Implementation Notes

### Audible beep

- **Where to implement**: In the audio capture pipeline, before the first audio frame from the system/microphone is written to the recording buffer. The beep should be synthesized (not a file read) for reliability.
- **Platform**: On macOS, use `AVAudioEngine` or the Tauri audio layer to play the beep through the active output device and simultaneously write it to the recording buffer.
- **Shared constant**: Define beep frequency (e.g., 1000 Hz), duration (e.g., 500 ms), and waveform (sine) in a config module.

### Bot identification

- **Meeting platform integrations**: When joining via a bot (if/when supported), set the bot's display name and description through the platform's API.
- **System audio recordings**: For local system-audio capture (the current architecture), the beep and visible indicator serve as the identification. No separate bot identity is needed.

### Visible recording indicator

- **Menu bar indicator**: June already has a menu bar presence. The recording indicator should be part of this — a red recording dot or pill in the menu bar icon or an overlay HUD.
- **Always-on-top**: The indicator must not be obscured. A menu bar item is always visible on macOS. Alternatively, a floating HUD (like the existing `meeting-hud.html`) can serve this purpose.
- **State management**: The recording state should be managed centrally so that any component can query whether recording is active and render the indicator accordingly.

### Consent language sharing

- **Calendar integration**: The most natural integration point is the calendar — add a note to the Google Calendar / Outlook invite when the user clicks "Share consent." This requires calendar API access, which may not be available at launch.
- **Fallback — clipboard / share sheet**: If calendar integration is not yet available, the one-click action should copy the consent language to the clipboard with a "Copied!" confirmation, or open the system share sheet. This still qualifies as "one-click" if the user's next action is a paste into an invite.
- **Template**: Store the default template in the app's configuration. Allow the user to edit it in Settings → Recording → Consent template.

### Default consent language template

Suggested default (editable by the user):

> This meeting will be recorded and transcribed by June, an AI notetaker by Open Software. The recording and transcript will be stored on the host's device and used to generate meeting notes. By joining, you consent to being recorded. If you have concerns, please contact the meeting host before joining.

## Out of Scope

- **Legal review of consent language**: The default template is a starting point; users should have their legal counsel review it for their jurisdiction.
- **Per-participant consent tracking**: Scribe does not track whether individual participants consented — the audible beep, visible indicator, and pre-meeting notice are the consent mechanisms.
- **Automated consent collection (e.g., a consent form before joining)**: Out of scope for v1; the one-click share + beep + indicator approach is the launch requirement.
- **Recording consent for dictation**: Dictation is a single-user feature (the user is recording their own voice); multi-party consent does not apply.

## References

- OpenSoftware Launch Checklist (Google Doc) — "Scribe Audio Recording Consent" section
- [`/docs/scribe-api-prd.md`](./scribe-api-prd.md) — Scribe API architecture
- [`/docs/onboarding-design.md`](./onboarding-design.md) — Meeting notes onboarding and permission flow
- [`/specs/001-tauri-note-mvp/`](../specs/001-tauri-note-mvp/) — Note MVP spec (recording foundations)
