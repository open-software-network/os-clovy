---
status: accepted
date: 2026-08-06
supersedes: 0054
---

# Clovy technical identity migrates through a compatibility bridge

## Context

ADR-0054 separated the Clovy presentation rebrand from June-era technical
identities because the latter bind released application state, credentials,
permissions, updates, and external clients. The product decision has now moved
one step further: Clovy should also become the canonical technical name, while
an upgrade must preserve every existing user's notes, recordings, memories,
agent sessions, sign-in, connector grants, linked devices, permissions,
extension connection, autostart, deep links, and updater continuity.

Those requirements rule out a blind rename. Some identifiers are ordinary
source names, some can move through a dual-read bridge, and some are identities
that macOS or Windows does not transfer. In particular, macOS TCC grants are
bound to signed bundle identities. No application migration can rewrite those
grants for a new bundle identifier. The updater endpoint and signing key baked
into a released binary are similarly permanent from that binary's perspective.

## Decision

### Clovy is canonical; June is compatibility

Repository-controlled names use Clovy whenever an old release does not need to
discover the new value directly. New code calls the products **Clovy**, **Clovy
API**, and **Clovy Companion**. Package, crate, module, script, workflow,
service, environment, header, native-host, and artifact names become
Clovy-canonical through the bridge described below.

A June-era value may remain only when it is one of the following:

1. an immutable operating-system or updater identity required to preserve an
   existing install;
2. an input alias accepted from a released client, extension, deployment, or
   operator environment;
3. a rollback output kept current by dual-write during the bridge window;
4. historical evidence, an append-only migration marker, or a shipped wire
   fixture; or
5. an external rename that has not yet completed with a verified redirect.

Every remaining value is documented as `legacy` or `compatibility`; it is not
the name used for new source concepts.

### Migration matrix

| Surface | Clovy-canonical identity | June compatibility contract |
| --- | --- | --- |
| Root JavaScript package | `clovy` | Lockfile history only |
| Agent harness package and executable | `@clovy/agent-runtime`, `clovy-agent-runtime` | Build environment accepts `JUNE_AGENT_RUNTIME_*`; released runtimes still terminate with their owning app |
| Browser extension package | `clovy-extension` | Store item and extension ID stay the same |
| Desktop Rust package and library | `clovy`, `clovy_lib` | Main executable remains `os-june` for updater and installed-path continuity |
| Windows dictation helper source package | `clovy-windows-dictation-helper` | Bundled executable remains `june-dictation-helper` for installed-path and rollback continuity |
| Windows helper composer identity | `clovyProcessId`, `clovyWindowHandle` | Desktop sends both field sets and the helper accepts `juneProcessId` / `juneWindowHandle` from released peers |
| Clovy API source, crates, and binary | `clovy-api/`, `clovy-*`, `clovy-api` | Deployment accepts `june-api` image/service aliases until every environment moves |
| Companion crates and C ABI | `clovy-companion-*`, `clovy_crypto_*` | Legacy crate consumers and `june_crypto_*` symbols remain forwarding aliases during the bridge |
| Desktop environment | `CLOVY_API_*`, `OS_CLOVY_*`, `CLOVY_AGENT_RUNTIME_*` | Read June-era variables as lower-precedence fallbacks |
| Clovy API environment | `CLOVY__SECTION__FIELD` | Merge `JUNE__*` first, then let `CLOVY__*` override it |
| Client version headers | `x-clovy-app-version`, `x-clovy-macos-version` | Clients send both; Clovy API accepts both; released fixtures keep the old headers |
| OS credential services | `co.opensoftware.clovy.*` | Migrate legacy-only values, then dual-write both so an application rollback retains rotated tokens and device identity |
| Browser storage | `clovy:*` | Migrate legacy-only values, dual-write both keys, and reconcile rollback-side changes from a last-synced marker |
| Share-viewer tab session | `clovy_share_state`, `clovy_share_token` | Read and dual-write `june_share_state` / `june_share_token`; use callback state to reconcile an in-flight PKCE request across deploy or rollback |
| Persona settings | `clovy-persona.json` | Dual-write `june-persona.json` and reconcile either side against a last-synced settings marker |
| Native messaging host | `co.opensoftware.clovy.extension` | Install both host manifests; the extension falls back to the legacy host |
| OAuth/deep links | `clovy://` | Register and accept `osjune://` until old login clients and callbacks age out |
| Release artifacts | `Clovy_*`, `Clovy-extension.zip` | Publish June-named aliases while old documentation and automation can still link them |
| API/image/domain/repository names | Clovy names after external provisioning | Preserve old DNS, GHCR, and GitHub redirects for every released caller |
| Persisted and `/v1` values | Clovy names only behind an additive alias or versioned contract | Existing error codes, JSON fields, database names, migration rows, action slugs, MIME values, and toolset ids remain accepted for as long as released data or clients use them |

### Idempotent bridge rules

- **Reads use provenance, then preserve the rollback-readable side.** A
  legacy-only value is copied to the canonical location. For divergent values,
  a last-synced marker identifies which side changed. If provenance is unknown,
  the June-era side wins because it is the only side a released rollback build
  can update. A failed copy never hides a readable source; the next read retries
  it. Because every bridge write publishes legacy first, canonical-only
  credential and browser-storage state means a rollback removed the legacy
  value; Clovy propagates that deletion instead of recreating it.
- **Writes update both identities during the bridge window.** This is
  load-bearing for rotating refresh tokens and connector grants: leaving the
  old entry untouched would make a rollback read an invalidated token. The
  rollback-readable side is published first; the last-synced marker lets Clovy
  recognize and promote that staged value after an interruption.
- **Credential divergence is resolved from a sync marker.** The bridge records
  a one-way fingerprint of the last value known to be present in both Keychain
  services. If a rollback build later rotates only the legacy credential, the
  next Clovy launch recognizes that change, promotes it, and refreshes both
  entries instead of overwriting it with stale canonical state.
- **Unknown credential provenance is never guessed.** A missing Keychain marker
  is safe because reconciliation preserves the rollback-readable side. A real
  marker read error aborts reconciliation without modifying either credential,
  and a marker write failure is reported only after both readable entries have
  reached the same value.
- **Browser-storage divergence follows the same rule.** A compact fingerprint
  records the last value observed on both keys. If a rollback build changes or
  removes only the June-era key, Clovy promotes that newer choice on the next
  read. If a Clovy write or delete stopped partway through, the same marker
  repairs the unfinished compatibility side.
- **Compatibility installs before persisted state is imported.** The browser
  bridge evaluates before App modules that capture storage values at module
  scope, including the active data partition.
- **Deletes clear both identities from the rollback side first.** Signing out
  or disconnecting must not leave a live credential under the alias the current
  UI no longer displays. Credential deletion tombstones every value observed on
  either side before Keychain cleanup, so a divergent stale value cannot be
  mistaken for a later login. Only a new value published through the
  rollback-readable service supersedes a tombstone. Once the tombstone commits,
  a one-shot secret is delivered even when physical cleanup remains pending.
- **Persona changes use the same rollback protocol.** The legacy file is
  written first, the canonical file second, and the full last-synced settings
  marker last. Re-upgrade preserves a persona changed during rollback.
- **Migrations are repeatable and crash-safe.** A process may stop after either
  side of a dual-write. The next launch converges both sides without deleting
  the source.
- **Wire changes are additive.** New clients send canonical and legacy headers;
  servers accept either. Existing `/v1` fields and error numbers do not change.
- **Old clients remain first-class during the bridge.** Clovy API, native
  messaging, release hosting, and deployment aliases keep serving released
  versions independently of the desktop rollout cadence.
- **Deployment supplies both environment namespaces.** Every production,
  staging, ephemeral, and link-viewer Compose definition derives equivalent
  `CLOVY__*` and `JUNE__*` values from one canonical-preferred expression.
  Promoting an older image with current deployment configuration therefore
  remains a supported rollback even while both host namespaces are present.

### Operating-system identities that remain June-era

The following values remain compatibility transport identities, potentially
for the lifetime of the application:

- macOS main, dictation, system-audio, and Computer use bundle identifiers;
- the installed `June.app` path and `os-june` main executable;
- the Tauri updater public key, legacy manifest endpoint, and any release URL
  baked into an existing binary;
- the Windows NSIS upgrade identity, installed `June` directory, and
  `os-june.exe` path; and
- the OS Platform product handle `june` and Issue prefix `JUN` unless the
  platform provides aliases that preserve every existing link and automation.

Keeping these values does not make them current product names. Finder, Windows
shell metadata, window titles, notifications, package descriptions, and all
other presentation continue to say Clovy. Any proposal to change an item in
this list must demonstrate an operating-system-supported transfer of the state
it owns; asking the user to re-grant a permission is not preservation.

### Rollout and retirement

1. Ship at least one stable **bridge release** that understands both identity
   sets, dual-writes rollback-sensitive state, and keeps all old service and
   release entry points alive.
2. Prove a released-June-to-Clovy update on macOS and Windows. The release gate
   checks notes, recordings, memories, sessions, OS Accounts, connectors,
   companion identity, permissions, extension connectivity, autostart, deep
   links, update/relaunch, and downgrade readability.
3. Move external DNS, container, repository, OAuth, and store configuration
   only after the canonical target exists and the legacy target redirects or
   remains published.
4. Retire an alias only in a later PR with evidence that no supported app,
   extension, deployment, stored row, or automation still uses it. Immutable
   operating-system identities are not retirement candidates.

Rollback from the bridge release is supported: durable data stays in the
existing bundle-scoped root, rollback-sensitive credentials and preferences
remain current under legacy keys, old API headers and endpoints still work,
and legacy artifacts remain published.

## Consequences

- New source and operational work uses Clovy names without abandoning released
  users or forcing a clean install.
- The codebase intentionally contains a smaller, auditable set of June strings
  labelled as compatibility values, historical records, or fixtures.
- Some low-level paths will never literally say Clovy because preserving their
  identity is the mechanism that preserves permissions and updater continuity.
- The bridge temporarily increases configuration, credential, artifact, and
  native-host test coverage because both identities must work.
- External provisioning is part of the migration and must complete before a
  release switches its canonical DNS or OAuth callback.

## Alternatives considered

- **Rename every identifier in one build.** Rejected because it creates a
  second app identity, loses TCC grants, strands credentials and data, and
  breaks released clients.
- **Keep every technical identifier forever.** Rejected because ordinary
  package and service names would remain misleading even where an additive
  compatibility bridge is straightforward.
- **Copy legacy state once and delete it.** Rejected because a crash can split
  the migration and a rollback would read stale or missing state, especially
  after refresh-token rotation.

Superada pela change desvincular-repo-ci na linha do ambiente do Clovy API: `JUNE__*` deixou de ser lido; o restante da ponte continua.

2026-10-21: superada pela [ADR-0068](0068-clovy-takes-its-own-identity-with-a-one-time-migration.md). O fork troca o identificador para `dev.guilhermexp.clovy`, copia uma vez o perfil e as credenciais June e remove a ponte.

## Addendum - 2026-10-28 (agent executable retired)

[ADR-0072](0072-cli-chat-replaces-the-native-agent.md) retires the agent harness
package/executable row above, including its build environment aliases and
owner-process lifecycle. Those names describe historical releases only; the
current desktop packages the MCP relay and runs installed CLI engines. This
addendum does not change identity migration or authorize deleting migrations.
