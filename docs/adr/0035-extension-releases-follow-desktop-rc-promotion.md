# ADR 0035: Extension releases follow desktop RC promotion

## Status

accepted - 2026-07-16, implemented in PR #744

## Context

ADR 0017 made the Chrome Web Store listing, publisher account, and review delay
part of Browser use's release surface. The extension can update independently of
the desktop app, but its native-messaging protocol and pinned extension ID mean
an arbitrary store package cannot be released independently without risking an
extension/app mismatch.

Chrome Web Store versions are one to four numeric components, each no greater
than 65535. They cannot express June's `X.Y.Z-rc.N` semver directly. Store
updates also undergo review, and API v2 supports deferred publication: a package
can be reviewed into `STAGED` and published later without uploading different
bytes. The older API v1 `trustedTesters` target is deprecated and is scheduled
for removal on 2026-10-15. A second private test listing would have a different
extension ID and therefore would not match June's native-messaging allowlist.

Release automation needs to answer four questions:

1. Which desktop source and RC does a store package belong to?
2. How do later desktop RCs avoid unnecessary store submissions?
3. How does stable promotion prove that Chrome reviewed the intended bytes?
4. How does CI authenticate without a long-lived Google service-account key?

## Decision

The extension joins the existing desktop RC-to-stable promotion transaction.
There is no independent stable extension workflow.

### RC submits for deferred publication

After `rc-desktop-release` publishes the desktop RC, an Ubuntu job checks out
the commit in `rc-build.json`, tests and builds the extension, and fingerprints
the normalized package payload before its release version is stamped. If that
fingerprint equals the latest stable extension metadata, the job records
`unchanged` and makes no Chrome Web Store write. It still verifies that the
expected version is published and no uncorrelated submission is active.
Unrelated lockfile or desktop changes therefore cannot trigger a store review
when the produced extension bytes are identical.

When the fingerprint changed, the job:

- maps `X.Y.Z-rc.N` to Chrome version `(X+1).Y.Z.N` and sets the display-only
  `version_name` to clean `X.Y.Z`; the major offset keeps the first automated
  release above the manual/bootstrap manifest version `0.1.0`;
- creates a deterministic `June-extension.zip` and records its SHA-256;
- uploads it with Chrome Web Store API v2;
- submits it with `publishType: STAGED_PUBLISH`, 100 percent deployment, and
  `blockOnWarnings: true`;
- writes `extension-build.json` beside `rc-build.json` on the fixed `rc`
  release.

The first numeric component is the desktop major plus one, and the fourth is the
RC iteration. Every replacement package therefore advances monotonically while
the user-visible version stays aligned to the desktop release. A later RC with
identical extension inputs reuses the existing submission and package only when
live store state confirms that exact version is still `PENDING_REVIEW` or
`STAGED`. Missing, mismatched, rejected, cancelled, or expired prior artifacts
are rebuilt with the higher RC version. A later RC with changed inputs may
cancel only the specific active version named by the prior RC metadata; an
uncorrelated active submission fails closed.

### Stable is gated on Chrome approval

`promote-desktop-release` freezes `rc-build.json`, `extension-build.json`, and
the extension ZIP into an immutable per-run GitHub artifact before any stable
desktop build starts. It verifies the metadata correlation and package hash.

If the extension changed, Chrome must report that exact store version as
`STAGED`. `PENDING_REVIEW`, `REJECTED`, a policy warning, a takedown, a different
version, a missing asset, or a mismatched hash blocks desktop promotion. If the
extension did not change, preflight verifies the expected published version and
refuses any uncorrelated active submission without writing to the store.

After the stable desktop release succeeds, a final job publishes the already
reviewed staged package with `DEFAULT_PUBLISH`. It then attaches the exact ZIP
and stable `extension-build.json` to the `vX.Y.Z` release. Publishing the
extension before the desktop is deliberately rejected: a desktop failure would
otherwise expose users to an extension that depends on an unreleased app.

Homebrew and source-repo version bookkeeping run only after that extension job.
They are deliberately outside the desktop-publication job so a bookkeeping
failure cannot suppress publication of the correlated extension. Unchanged
bytes are rechecked against live store state after the desktop publish too.

The desktop release is assembled as a draft. Every expected asset is uploaded
and verified against SHA-256 values in `stable-build.json`. The signing job then
freezes that provenance as an immutable Actions artifact before a separate job
can publish the draft and mark it latest. Publication and Homebrew both verify
downloaded release assets against that earlier artifact; neither can bootstrap
trust from the mutable release copy of `stable-build.json`. A lost publication
response is retried in the original workflow run so the same artifact remains
the authority. A new promotion run refuses an already-public release.

The RC and stable-promotion workflows share one non-cancelling concurrency lock,
so a newer fixed RC release cannot replace a candidate during promotion. The
two external stores cannot be updated atomically. The desktop therefore
publishes first; an extension publish failure leaves the workflow red and is
safe to retry because the store operations are idempotent.

On the fixed RC release, the ZIP is uploaded (or the obsolete ZIP removed)
before `extension-build.json`. Metadata is the commit marker: promotion never
trusts a package write that did not complete. A retry rebuilds deterministically
and lets the current expected version perform the final idempotency check.

### CI uses keyless identity federation

GitHub Actions authenticates through GitHub OIDC, a Google Workload Identity
Provider, and a dedicated service account registered with the Chrome Web Store
publisher. The workflow requests a short-lived access token scoped only to
`https://www.googleapis.com/auth/chromewebstore`. No JSON service-account key,
OAuth refresh token, or publisher credential is stored in GitHub.

All store-writing jobs use the protected `production` GitHub Environment. The
Workload Identity Provider must restrict its trust to the
`open-software-network/os-june` repository and that environment subject.

## Considered options

- **Publish every extension change directly to stable** - rejected because
  Chrome review would be asynchronous with the desktop release and no tested
  promotion boundary would remain.
- **Use API v1 `trustedTesters` for RC** - rejected because v1 is deprecated and
  has a fixed removal date. New release infrastructure must use API v2.
- **Maintain a second private store item** - rejected because it has a different
  extension ID, requiring a second native-messaging allowlist and creating a
  test path that is not byte-identical to production.
- **Rebuild or re-upload at stable promotion** - rejected because it would send
  different bytes through a second Chrome review and would not prove that the
  promoted package is the reviewed RC.
- **Store a service-account JSON key or OAuth refresh token** - rejected because
  GitHub OIDC and Workload Identity Federation provide short-lived credentials
  with a narrower trust boundary and no rotation burden.

## Consequences

- A Chrome review can delay the whole stable release. This is intentional: the
  extension/app compatibility boundary is more important than release speed.
- RC means "submitted for deferred store publication," not a second installable
  Web Store channel. Developers still test the same build unpacked during the
  RC soak; Chrome's review validates the package that will become public.
- `extension-build.json`, `rc-build.json`, and `June-extension.zip` are
  load-bearing release evidence. Promotion fails rather than reconstructing or
  trusting missing metadata.
- Store versions remain monotonic and may visibly include a fourth numeric
  component only in technical surfaces; `version_name` stays clean.
- Store listing/privacy changes remain dashboard operations. After a visibility
  change, Chrome requires one manual publish before API publishing works again.
- A staged submission expires after Chrome's deferred-publication window. If it
  expires, cut a higher desktop RC rather than bypassing the gate.

## 2026-08-12 addendum: source repository rename

The source repository moved to `open-software-network/os-clovy`. Before
extension publication is enabled, the Workload Identity Provider must trust
that repository and the production-environment subject documented in
`docs/release-extension.md`. The earlier `os-june` subject above describes the
repository coordinate in effect when this decision was accepted.

Superada pela change desvincular-repo-ci.
