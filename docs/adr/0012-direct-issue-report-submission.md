# ADR 0012: Direct issue-report submission with server-side diagnosis

Date: 2026-07-06
Status: accepted

## Context

Issue reports (bug, feedback, feature) were composer messages tagged with a
category chip. Submitting one wrapped the user's text in an investigation
prompt and ran it as a normal agent turn before the report was delivered to
`/v1/issue-reports` with the resulting `agentDiagnosis`. Three code comments
described a server-side "no-charge waiver" keyed off the report category, but
no waiver was ever implemented in June API or OS Accounts: the investigation
turn was metered and charged like any agent chat, at whatever generation model
the user had selected (JUN-213). Users were paying to file issue reports, and
the price scaled with their model choice.

Making the intended waiver real was designed first and rejected. The waiver
signal has to originate in the client (only the composer knows a turn is a
report), and the turn's model calls flow through the embedded Hermes runtime,
whose `prompt.submit` contract carries no metadata — the flag would have to
ride a synthetic model id or per-session state in the shell's loopback
provider endpoint (the Bridge), then be honored by June API only under a
server-enforced model pin so a spoofed flag could not mint free tokens on an
expensive model. That is three layers of new contract (frontend, the shell's
loopback endpoint, June API) to keep an investigation turn nobody sees,
and it strands every already-shipped app version, which sends no flag and
would keep charging users.

## Decision

Reports no longer run a client-side model turn at all.

- The client submits reports directly: a dialog (reachable from the composer
  "+" menu, the sidebar, and settings) collects category, description, and
  file attachments, and calls `submit_issue_report` -> `/v1/issue-reports`.
  No Hermes session is involved, so there is nothing to authorize or charge.
  The category chip machinery stays wired for drafts that still carry chips,
  but no entry point inserts new chips.
- The team-facing diagnosis moves into June API. When a report arrives without
  an `agentDiagnosis` (new clients) and `issue_reports.diagnosis_model` is
  configured, June API runs one non-streaming completion on that model at
  June's own upstream expense — never metered, never charged, invisible to the
  user — under a hard timeout, and feeds the text through the existing
  diagnosis splitter. Any diagnosis failure delivers the report undiagnosed;
  diagnosis can delay a submission by at most the configured timeout and can
  never drop one. Old clients that still send a client-side diagnosis pass
  through unchanged.

## Consequences

- JUN-213 is fixed by construction for updated clients: no model call exists
  on the report path, so no waiver, model pin, or anti-spoofing logic is
  needed. Not-yet-updated clients keep today's charged behavior until they
  update; June API cannot waive their turns because they send no signal.
- June eats one cheap, server-pinned completion per report instead of users
  paying for one user-priced turn per report. Cost is bounded by config
  (`diagnosis_model`, `diagnosis_timeout_secs`); unsetting the model turns
  diagnosis off entirely.
- The server-side diagnosis sees only the report fields (category,
  description, platform, app version, attachment names), not the user's
  session context or attachment bytes, so it is shallower than the old
  client-side investigation. Richer inputs (attachment text, screenshots)
  are a deliberate follow-up.
- The report UX is fire-and-forget (JUN-197 direction): no visible agent
  conversation, no user-facing diagnosis. If a report-followup conversation
  is wanted later, it needs a new design.

## Addendum: 2026-07-10

### Decision

- Attachments stay on the authenticated desktop -> June API -> os-platform
  path. The desktop streams every readable file that fits a 300 MiB per-file
  and cumulative budget; June API accepts 1 MiB of additional multipart
  overhead and forwards every received file to os-platform. New clients send
  `stream=true` and accept a heartbeat SSE response whose terminal result or
  error contains the same envelope as the original buffered JSON response.
  Old clients omit the flag and keep the buffered contract, while new clients
  still accept JSON from an older server. One deadline created before permit
  wait and multipart extraction bounds the complete request and downstream
  delivery to `request_timeout_secs`. The desktop's dedicated issue-report
  request window is longer (900 seconds with the 600-second server default),
  leaving transport and terminal-response grace so the server remains the
  authoritative deadline.

### Consequences

- A single issue-report permit spans multipart extraction, downstream
  delivery, and streamed response completion. This bounds June API to one
  platform-sized attachment buffer at a time, including when a client
  disconnects while accepted delivery finishes in the background.
- File and Issue creation requests are non-idempotent. June API does not
  replay ambiguous transport or malformed-envelope failures. The legacy
  project-to-org Issue fallback is retained only after a parsed, explicit
  project rejection proves the first create did not succeed.

## Addendum: 2026-07-31

### Decision

- A bug report opened from a stored agent session may include a generated
  `june-agent-diagnostics.txt` attachment. The dialog shows this attachment
  before submission and lets the user omit it. Feedback and feature reports
  do not attach agent diagnostics.
- Diagnostics are limited to the latest run in that stored session, and are
  attached only when that run failed. They contain qualified stored session
  and agent run IDs, a bounded stable error code, and completion time.
  They do not contain prompts, transcript content, tool output, or user files.
- The desktop does not transmit the persisted free-form error message. It
  bounds and sanitizes the identifiers and stable error code before the
  existing June API attachment contract carries the generated text, so
  diagnosis remains server-side and no backend deployment is required.
- Diagnostic lookup is best-effort. Database or migration failures are logged
  without report content, and the user-authored report is still submitted.

### Consequences

- The team receives actionable local failure context without asking a user to
  run a command or collect logs.
- Sending a report can disclose bounded internal identifiers, a stable error
  code, and completion time, but the dialog makes that disclosure visible and
  removable.
- Report payloads stay within the server's 20-attachment budget. The dialog
  accepts up to 19 user-selected files while generated diagnostics are selected.

## Addendum: 2026-08-06

Under ADR-0055, newly generated diagnostics use the canonical
`clovy-agent-diagnostics.txt` attachment name. The June-named attachment above
describes the historical implementation and is not a persisted or accepted API
identifier.

Superada pela change desvincular-repo-ci no encaminhamento a os-platform: Clovy API só registra os relatórios em log; a submissão direta e o diagnóstico no servidor continuam.
## Superada pela change desvincular-servicos-open-software

Neste fork o app desktop não envia mais relatórios de erro: o ReportDialog e as chamadas a `/v1/issue-reports` foram removidos (2026-10-21).
