# Venice Content Filter — Launch Checklist Spec

**Created**: 2026-06-12
**Status**: Implemented
**Source**: OpenSoftware Launch Checklist (Google Doc)

---

## Overview

Venice AI provides a content filter that screens inference requests and responses for illegal activities. The filter is already implemented and active in the June/Scribe inference pipeline. This spec documents the requirement, the implemented behavior, and the response mechanism for flagged content, to serve as a reference for the launch checklist and future audits.

## Requirements

### REQ-1: Content filter for illegal activities

All Venice-backed inference calls must pass through Venice's content filter, which detects requests or responses involving illegal activities.

**Acceptance Criteria**:

1. Every inference request routed through Scribe API or directly to Venice includes the content filter (enabled by default in the Venice API).
2. Requests flagged by the filter are blocked before the response reaches the user.
3. The filter applies to all modalities: agent chat, dictation cleanup, note generation, and model listing.

### REQ-2: Response mechanism for flagged content

When the content filter flags a request or response, the system must handle it gracefully and, where applicable, trigger account-level actions.

**Acceptance Criteria**:

1. **User-facing response**: When a request is filtered, the user receives a clear, non-judgmental message indicating the request could not be completed (e.g., "I'm unable to help with that request"). No details about the filter or the specific policy violated are exposed to the user.
2. **Logging**: Filtered requests are logged server-side with the user ID, timestamp, and a categorization of the filter trigger. Logs are not accessible to end users.
3. **Reporting**: Repeated filter violations from the same account are reported to the platform moderation team for review.
4. **Account suspension**: If the platform moderation team determines an account is systematically attempting to generate illegal content, the account may be suspended per the OS Accounts terms of service. Suspension is a manual, human-reviewed action — not automatic.

### REQ-3: No bypass path

There must be no supported way for a user to disable or bypass the content filter.

**Acceptance Criteria**:

1. The filter cannot be disabled via settings, environment variables, or API parameters.
2. Direct API calls to Venice (bypassing Scribe API) from the client are not possible — the client holds no Venice API keys (see [`scribe-api-prd.md`](./scribe-api-prd.md)).

## Implementation Notes

- **Current state**: The Venice content filter is enabled by default on all Venice API endpoints. No client-side configuration is required; the filter runs on Venice's infrastructure.
- **Error handling in Scribe API**: When Venice returns a content-filtered response, Scribe API maps it to an appropriate error code in the `ApiResponse` envelope. The Tauri client displays a user-friendly message and does not surface the raw error.
- **Reporting pipeline**: Filtered-request events are emitted as structured `tracing` logs. A downstream alerting or reporting system consumes these logs to flag accounts for moderation review. The exact reporting infrastructure (dashboard, alerts) is out of scope for this spec but tracked as a follow-up.
- **Suspension flow**: Account suspension is handled through OS Accounts, not through Scribe or Scribe API. Scribe API does not have the ability to suspend accounts; it can only report.
- **OpenAI path**: OpenAI's moderation API provides equivalent filtering for requests routed through OpenAI. This spec focuses on the Venice path per the launch checklist, but the principle (filter + report + suspend) applies to all upstream providers.

## Out of Scope

- Building a custom content filter — Venice and OpenAI provide this; we use theirs.
- Defining the specific categories of illegal content — that is determined by the upstream provider's policies.
- Automated account suspension — suspension is a human-reviewed action.
- Content filter for on-device / local inference (if ever added) — not part of the current architecture.

## References

- OpenSoftware Launch Checklist (Google Doc) — "Venice Content Filter" section
- [`/docs/scribe-api-prd.md`](./scribe-api-prd.md) — Scribe API architecture and error mapping
- [`/docs/os-accounts-backend.md`](./os-accounts-backend.md) — OS Accounts integration
