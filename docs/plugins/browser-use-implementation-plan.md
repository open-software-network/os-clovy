# Implementation plan: Browser use plugin

Updated 2026-11-01. The approved OpenSpec change
`melhorias-agente-audio-automacao`, design D6 and `browser-control` delta own the
cutover scope. The [detailed PRD](../browser-computer-use-prd.md) owns the
13-tool contract and trust boundary. Historical ADR-0017 is retained as history;
its extension/native-messaging transport is replaced by the integrated engine.

## Architecture

```text
Bound CLI turn / authorized routine
  -> Clovy MCP browser group (13 tools)
  -> native browser broker + central approvals
  -> authenticated private typed host
  -> pinned agent-browser engine + Chrome for Testing
       | attended: exclusive separate persistent profile
       | routine: anonymous ephemeral profile + public-network policy
```

The host creates session ownership, namespace, profile and lifecycle state.
Models cannot choose or override them. Every call rechecks ownership and grants,
and approvals revalidate after waiting. Tab ids are integers identifying exact
owned targets. Snapshot references include document/epoch identity; guarded
input checks the exact origin/document/node/element, without name recovery.
`fill` uses `text`; `press` requires its target `ref`; `back` requires `tab_id`.
See the detailed PRD for the complete required-argument table.

## Delivery slices

1. Build the pinned downstream engine with authenticated private typed host,
   exact-reference guards and disabled public/streaming/plugin paths.
2. Wire the production transport at startup, artifact/outcome roots, access
   flag and cancellation/shutdown supervision from session reservation.
3. Lease the attended profile exclusively until both processes exit; retain
   logins on Stop. Isolate routine profiles, grants and DNS/IP-pinned network.
4. Connect host UI grant/session commands, mode display, human takeover and
   explicit resume. Suspend captures and actions during human login.
5. Remove extension package, pairing/share-tab APIs and old transport. Package
   pinned engine/browser resources with hashes and provenance.
6. Verify the real adapter/engine with synthetic pages and frontend controls;
   record native macOS/Windows and genuine external-login acceptance separately.

## Failure and privacy

Background is default. Visible mode can be selected before an effect or after
an explicit unsupported/not-attempted result, never as replay after delivered
or uncertain input. Cancellation invalidates authority and reaps engine/browser
processes; no startup orphan or premature profile-lease release is acceptable.
Anonymous startup recovery and Stop/reaper now share the physical collector:
existing exclusive lease, durable `Starting` cancellation, original-tree and
boot checks for `Owned`, and a locked root terminal record before controls are
retired. Unix uses a physical holder; Windows pins the original UUID directory
in place because open descendants prevent its rename. The version-2 terminal
location explicitly distinguishes these placements. The same collector retires
controls and empty directories under the locked terminal; only then may Windows
release the pin and acquire DELETE by the same physical handle. Cancelled
publication closes/replaces/revalidates its receipt under the original lease.
Recovery does not kill or replay. Unknown/legacy/corrupt objects and
unexpected terminal payload are preserved. The browser ADR's 2026-11-02
addendum documents the holder/terminal format and platform limits.
Closed tabs and stale references fail explicitly. Saved logins never revive
old handles. No generic upstream MCP, arbitrary eval, personal-browser attach,
profile/cookie import or sensitive-field automation is exposed.

Only synthetic, run-owned data is used for automated proof. Native packaging,
permissions and actual provider login require native acceptance. Browser-shim
screenshots establish frontend rendering only.
