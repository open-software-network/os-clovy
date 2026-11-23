# PRD: Browser use plugin

Updated 2026-11-01 for the approved `melhorias-agente-audio-automacao` change.
The [detailed contract](../browser-computer-use-prd.md) owns tool names, policy,
transports and verification. This portfolio summary owns the product promise.

Browser use lets Clovy finish interactive web tasks that have no structured
connector. Clovy uses its own separate browser and persistent login profile.
It works in the background by default, with a visible window when an operation
cannot run in the background. It never copies or attaches to the user's usual
browser. Connectors remain preferable for narrow, structured workflows.

Users enable Browser use in Plugins or through the in-chat consent card. The
copy explains that page text and screenshots may reach the configured AI model.
No extension installation, pairing, store listing or tab-sharing code is needed.

Login is a separate human step. Take over pauses every agent capture and action;
the user signs in and explicitly resumes agent control. Sensitive password,
one-time-code and payment fields stay human-only. Stop closes browser control
and processes while preserving the saved login. The UI shows the effective
mode and whether control is suspended. Consequential actions remain subject to
the native approval policy, and uncertain deliveries never retry automatically.

Authorized routines use a separate anonymous ephemeral profile and their own
per-routine opt-in. They never inherit attended logins or approvals, and
clicks, fills and key presses remain blocked. Page labels cannot prove that an
input event is harmless; attended input uses the central approval broker.

Success means a user-confirmed or broker-verifiable task outcome, clear consent,
zero unauthorized actions, correct isolation and reliable Stop. An agent's
claim of completion alone is not evidence. Risks include changing page content,
stale targets, prompt injection, private network access, browser process
lifecycle and packaging pinned engine/browser resources across platforms.

Clovy is account-free and calls the user's configured providers directly.
Historical subscription-tier and extension-store assumptions do not apply to
this fork. Native acceptance and genuine provider login remain distinct from
synthetic fixture proof.
