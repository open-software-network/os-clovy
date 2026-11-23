# Companion APNs setup

> **Historical document (2026-10-28).** Server, Auto/funding, hosted relay and
> native-agent claims below describe an earlier design, not the current app.
> The inference server is removed; chat/routines use installed CLI engines,
> Groq/OpenRouter/fal operations use the user's keys directly, and sharing has
> no replacement. Current boundaries are in
> [ADR-0073](adr/0073-desktop-calls-user-providers-without-a-clovy-server.md),
> [llm-providers.md](llm-providers.md), and [configuration.md](configuration.md).


> **Fork note:** this fork's desktop app no longer has the companion (removed by
> the `desvincular-servicos-open-software` change). The Clovy API relay and the
> `crates/clovy-companion-*` / `june-companion-*` crates are removed in a later change.

1. Register `co.opensoftware.june.companion` in the Apple Developer portal and
   enable Push Notifications and Background Modes remote notifications.
2. Create an APNs token signing key and keep the `.p8` file server-side only.
3. Provision development/distribution profiles carrying `aps-environment`.
4. Set `CLOVY__COMPANION__APNS_TEAM_ID`,
   `CLOVY__COMPANION__APNS_KEY_ID`,
   `CLOVY__COMPANION__APNS_PRIVATE_KEY_PEM`,
   `CLOVY__COMPANION__APNS_BUNDLE_ID`, and
   `CLOVY__COMPANION__APNS_PRODUCTION` on Clovy API.
5. Use sandbox APNs for development builds and production APNs for TestFlight
   and App Store builds.

iOS registers the binary token natively and sends it only to the
device-credential-authenticated linked-device endpoint. The SwiftUI application
model never receives it. Registration does not request alert, badge, or sound
permission because the companion does not send visible notifications. When an encrypted frame
targets an offline phone, the relay sends `{ "aps": { "content-available": 1
} }` with background push type and priority 5. There is no visible or sensitive
notification content. If iOS grants background execution while the Keychain is
available, the companion refreshes its encrypted cache, returns to the locked
state, and disconnects. The linked configuration, device identity, device
credential, cache key, and encrypted cache file use the after-first-unlock
protection class required for that background path; the app still requires
device-owner authentication before displaying the cache. Native transport
requests honor task cancellation, and the background refresh cancels after 20
seconds so the app reports no data instead of overrunning its wake window. iOS
may decline to wake the app; foreground reconnect and resynchronization must
still work. Existing credentials created with the older when-unlocked access
class are migrated after the next successful Keychain access.

Do not add PushKit, VoIP claims, background audio, notification content, or an
APNs key to the mobile bundle. Remove invalid/unregistered device tokens during
operations follow-up when Apple returns those statuses.
