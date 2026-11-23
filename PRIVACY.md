# Clovy privacy

Recordings, transcripts, notes, and session history are stored on-device.
Audio or text leaves the device when a requested operation uses a provider:
Groq for transcription, OpenRouter for remote notes and optional dictation
cleanup, and fal for images/video. These calls use your own key directly.
With Local (Parakeet Ultra) selected, dictation, note transcription, turn
packs, retries, and live preview process audio on-device, without transcription
requests or fallback to Groq. Installing the optional model downloads pinned
files from Hugging Face; those downloads send no audio or provider key.
Optional text cleanup and note generation still use their separately selected
provider or CLI and may send the resulting text off-device.
CLI chat and CLI note generation use the CLI's account; configured endpoints
receive the content sent to them. Provider retention depends on that route.

Clovy sends no usage telemetry or issue reports, requires no Clovy account,
and offers no sharing service. There is no Clovy inference server or local
bearer token. Keys are stored in the operating system credential store and
sent only to their own provider. Media downloads carry no provider credentials.
Clovy's local MCP tools retain their turn-bound approval and data boundaries.

## Meeting video

On macOS, you can opt in to recording the detected meeting window. This is
off by default. The video helper captures only that window, without audio,
and requires its own Screen Recording permission. Microphone and system
audio continue through the separate audio pipeline. Pausing the recording
also pauses the video.
The desktop's separate Screen Recording grant is needed to enumerate meeting
windows. Without access to those window details, the video option is hidden.

Video is stored locally beside the session's audio as `window.partial.mp4`
during capture and `window.mp4` after finalization. Deleting the recording or
its note deletes the video. Video can use substantially more disk space than
audio. A crash may leave the last incomplete fragment unrecoverable.

When Notes uses the Claude CLI, Clovy can run your installed `video-analysis`
skill with local Python and FFmpeg. Extracted frames and transcript content
are then processed by Claude through your configured Claude service. The per-call
temporary evidence directory is deleted after analysis, and Claude session persistence is
disabled. Clovy does not bundle the skill, Python or FFmpeg.

Other Notes engines and unsuccessful analysis use only the transcript, with
a notice explaining that the video was not analyzed. Turning video on does
not enable full-display capture or tools in other one-shot generation calls.
