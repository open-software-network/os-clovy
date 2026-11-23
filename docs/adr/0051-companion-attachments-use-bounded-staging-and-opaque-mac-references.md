---
status: accepted
date: 2026-07-28
---

# Companion attachments use bounded staging and opaque Mac references

## Context

June Companion needs to attach a file created on the phone and to select an
existing file from the Mac. Treating either feature as generic filesystem
access would collapse the companion trust boundary: a stolen linked phone
could read arbitrary Mac files, a hostile prompt could walk outside a selected
directory, and an interrupted upload could duplicate bytes or agent turns.

The relay remains blind and accepts only bounded encrypted frames. A single
frame cannot carry a useful attachment, and SQLite, staged files, frontend
agent dispatch, and the agent workspace do not share one transaction. ADR-0048
therefore still governs every mutating begin, chunk, commit, and agent-send
operation.

## Decision

- Add separate `filesUpload` and `filesBrowse` capabilities. Body-to-capability
  equality is validated before dispatch. Neither capability implies the other
  or grants a general path, file-content, shell, Tauri, or runtime operation.
- A phone attachment uses begin, chunk, and commit requests under one
  phone-generated upload reservation id. The begin declares a bounded UTF-8
  name, optional bounded media type, exact byte count, and lowercase
  hexadecimal SHA-256 digest. Each mutating frame also carries its ordinary
  stable operation id and follows ADR-0048.
- One file is at most 25 MiB. One raw chunk is at most 32 KiB, upload offsets
  must be contiguous, a linked device may hold at most four active
  reservations and 50 MiB of staged bytes, and a reservation expires after one
  hour. A duplicate reservation with identical metadata resumes; conflicting
  metadata is rejected.
- `filesUpload` is part of the fixed capability set for every active linked
  device, including devices linked before this capability shipped. New pairing
  approval and each linked-device card disclose that device's upload
  capability; unlinking that device is the revoke control. This implicit grant
  is acceptable in v1 because uploads are owner-only, untrusted, non-executable,
  limited to four reservations and 50 MiB per device, expire after one hour,
  and cannot affect an agent until the device explicitly attaches a committed
  reference to a message.
- Uploaded bytes land under June's app-data directory, partitioned by an
  account digest, linked device id, and reservation id. Caller-supplied names
  never become path components. Chunks are written atomically with owner-only
  permissions, commit checks the declared size and digest before an atomic
  final rename, and incomplete, consumed, revoked, or expired data is removed
  best effort.
- The media type and file extension are advisory. June does not execute,
  preview, auto-open, or grant executable permission to uploaded bytes. A
  committed upload is an untrusted agent attachment and reaches the selected
  agent run through the same June-owned workspace copier used by the desktop
  composer. Malware scanning and type-specific content validation are deferred
  from v1 and must not be claimed.
- Mac browsing starts only from roots the signed-in user grants with the native
  directory picker in June Desktop Settings. Grants are persisted per OS
  Accounts user and can be revoked individually. No home, Desktop, Documents,
  cloud-storage, or volume root is granted implicitly.
- The encrypted protocol exposes `browseRootsList`, paginated `browseDirList`,
  and `browseFileStat`. Roots have opaque ids and bounded display labels.
  Requests use bounded relative path components. Responses contain bounded
  names, entry kinds, sizes, and modification times. `browseFileStat` may mint
  a short-lived opaque attachment reference. There is no `fileRead`,
  byte-range read, download, write, move, delete, or directory mutation in v1.
- A granted root is canonicalized before persistence. Each request rejects
  absolute paths, parent traversal, platform prefixes, hidden components,
  symlinks, non-UTF-8 names, and non-regular targets. It canonicalizes the
  current root and target, proves the root has not been replaced and the target
  remains beneath it, and repeats those checks when an attachment reference is
  consumed. The persisted Unix filesystem device id and root-directory inode
  must also match on every use, so a different volume remounted at the same
  path fails closed as a changed root. A legitimate filesystem identity change
  also fails closed and requires the user to remove and re-add the grant.
  Directory pages never follow symlinks.
- Agent messages carry at most eight opaque attachment references. The
  controller resolves them only for the authenticated account and linked
  device, rejects expired or revoked references, and passes local paths only
  to the trusted desktop frontend intent. The phone and relay never receive an
  absolute Mac path. Uploaded files are removed after the agent runtime has
  copied them or when their one-hour lifetime ends; browse references expire
  after 15 minutes.
- The 44 KiB plaintext frame ceiling remains unchanged. A 32 KiB chunk expands
  to 43,692 base64 characters. The versioned frame, UUIDs, tags, offsets, and
  JSON punctuation fit in the remaining 1,364 bytes; a contract test encodes
  the worst-case request and keeps this arithmetic honest. Noise overhead fits
  below the existing 45 KiB ciphertext ceiling, whose base64 relay envelope
  remains below 64 KiB.

## Consequences

The phone can contribute bytes and choose a Mac-side file for an agent turn
without receiving a remote file browser or content-download primitive.
Granting a root intentionally discloses its visible file names and metadata to
that linked device over E2EE, so Settings must describe and revoke that grant
clearly.

Large transfers require many online frames and cannot continue while the Mac is
offline. A failed or crashed mutation can remain outcome-unknown under
ADR-0048; a later explicit action may resume an identical upload reservation
without applying a chunk twice. The implementation carries cleanup and
revalidation work that a generic path API would avoid, but it preserves the
desktop authority and keeps relay/API changes unnecessary.

Existing linked devices gain the bounded `filesUpload` capability on upgrade.
Settings makes that grant visible per device and unlinking is the v1 revoke
mechanism; a narrower per-device capability toggle is deferred.

## 2026-07-28 addendum: browse staging survives only its reference lifetime

Browse attachment copies live in a Desktop-owned staging namespace, but their
references are intentionally memory-only. Cleanup therefore cannot rely only
on that map: Desktop scans the staging namespace periodically, preserves every
currently tracked reference, and removes untracked directories once the
15-minute reference lifetime has elapsed. This covers process termination and
restart without racing a newly materialized or still-live reference. A failed
materialization removes its staging directory immediately when possible.

## Superada pela change desvincular-servicos-open-software

Neste fork o app desktop não tem mais o companheiro (pareamento, transporte, aprovações remotas do Computer Use e a aba "Linked devices" foram removidos em 2026-10-21). O relay do Clovy API e os crates `crates/clovy-companion-*`/`june-companion-*` ainda existem e saem numa change futura.
