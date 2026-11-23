# PRD: Private connectors & away-mode relay

> **Historical document (2026-10-28).** Server, Auto/funding, hosted relay and
> native-agent claims below describe an earlier design, not the current app.
> The inference server is removed; chat/routines use installed CLI engines,
> Groq/OpenRouter/fal operations use the user's keys directly, and sharing has
> no replacement. Current boundaries are in
> [ADR-0073](adr/0073-desktop-calls-user-providers-without-a-clovy-server.md),
> [llm-providers.md](llm-providers.md), and [configuration.md](configuration.md).


**Owner:** CEO · **Date:** 2026-07-09 · **Status:** Draft for review
**Companion doc:** [private-connectors-implementation-plan.md](private-connectors-implementation-plan.md) (CTO)
**One-liner:** Give Clovy Town-grade integrations — an assistant that acts in your email, calendar, and work tools, proactively — without OpenSoftware ever being able to read user data.

> **Hosting note (2026-10-21).** Away mode assumed the TEE-attested Clovy API
> on Phala, reproducible builds and `/verify`. The `desvincular-repo-ci` change
> removed that hosted infrastructure: Clovy API now runs only locally. Local
> mode is unaffected; away mode has no hosting target until a new one is
> decided.

---

## 1. Why now

### The market just told us what wins

Town (town.com) raised a $55M Series A led by a16z on Clovy 3, 2026 — five weeks after we launched Clovy — selling exactly one thing Clovy doesn't do yet: an assistant that *executes* in your inbox and calendar. Their single most important number: **99% two-month retention for users who build at least one custom routine** (company-reported, but directionally credible). The lesson is not "email assistant" — it's that **routines running on your real work are the retention primitive for this category.** Chat is a visit; a morning briefing is a habit.

### The window

Town's architecture is their weakness: the whole product only works if you hand your inbox, calendar, and documents to their US cloud (Convex/AWS), protected by policy promises. They know it — they've publicly said they're investing in "zero-knowledge architecture." That means we have a window, not a permanent moat. We win by shipping the Town experience from the private side before they ship the private story from the cloud side. Every quarter of delay shrinks the differentiation.

### Why we win this if we move

- **We already own the hard part.** Local agent, routines engine, approval pipeline, TEE-attested backend, reproducible builds, `/verify`. Town would have to *rebuild their product* to match our architecture; we only have to *add connectors* to match their features.
- **The agent already lives inside the user's context.** Town mirrors your life into their cloud to learn you; Clovy already has the user's local notes, transcripts, and files available through its existing context tools, so connector routines can build on that foundation immediately.
- **We unlock the customers Town structurally cannot serve** — lawyers, clinicians, finance, HR, journalists — professions where "your assistant reads everything" is only acceptable if nobody else can.

## 2. Who it's for

**Primary: the confidential prosumer.** Solo or small-team professionals whose work *is* sensitive information — attorneys, accountants, therapists, recruiters, journalists, founders under NDA. They want Town (they've seen the demos) and cannot defensibly use it. Today they under-use AI or leak data guiltily. Clovy with connectors is the first assistant their professional obligations actually permit.

**Secondary: the privacy-conscious operator.** Our current base — technical, Mac-native, already sold on local-first. Connectors turn Clovy from "private chat + dictation + notes" into their daily operating layer, and they're the evangelists who validate the architecture publicly (they read the threat model; their approval is the trust signal the primary audience relies on).

**Explicit non-target (v1):** teams/orgs. Individuals first; the team layer is a separate PRD once individual retention proves out.

## 3. What we're building

### 3.1 Connectors — private by architecture, not policy

Gmail and Google Calendar at launch; Slack, Notion, Linear in the fast-follow. Two modes, and the user always knows which one they're in:

- **Local mode (default).** You authorize Google *on your Mac*; the keys stay in your Mac's Keychain; every provider API call comes from your device. OpenSoftware's servers are not in the *connector* data path — we hold no credential that could read your mail even under compulsion. Model inference for routines is a separate, existing path: connector-derived prompts run through the user's selected provider — the TEE-attested Clovy API by default, or fully on-device with a local model — and the claim copy must say so (the "not in the data path" line covers token custody and provider calls, never inference). This covers the large majority of real usage.
- **Away mode (opt-in, Pro).** For "works while your laptop is closed": events route through the same attested enclave that already runs Clovy API. Nothing is ever stored in plaintext; keys are sealed to published, verifiable code; queued items are encrypted to your device and deleted on delivery. The trust surface grows by exactly three named things (Intel TDX, Phala key management, our upgrade governance) and we say so out loud on a published threat-model page.

**Product principle:** privacy claims are downstream of the threat-model page, never ahead of it. No absolutes ("we can never see anything") — the honesty *is* the positioning, same discipline as our existing claims guardrails.

### 3.2 Execution — the assistant finally acts

Triage, drafting in your voice, scheduling, invite responses — with **trust modes** on every routine, copied shamelessly from Town's proven UX and improved:

- **Read-only** → **Approval-required** (default) → **Autonomous.**
- Our twist: **earned autonomy.** A routine can only go autonomous after it has run correctly under approval a few times. This converts our honesty brand ("the agent can make mistakes") into a mechanic instead of a disclaimer — and it's a demo-able differentiator.
- Approvals are batched and fast (approve five drafts at once), because approval fatigue is what pushes users to unsafe autonomy.

### 3.3 Routines gallery — the retention engine

Named, one-tap templates on the existing routines engine: **Morning briefing**, **Auto-inbox**, **Meeting prep** (which composes with our existing meeting notes: "here's what happened last time you met these people" — something Town cannot do without recording your meetings into their cloud). Activation bar: **first routine live within 10 minutes of connecting Gmail**, first run executes immediately so value is visible in the first session.

### Out of scope (v1)

iOS companion app (fast-follow; needs the relay), team/org layer, in-enclave routine execution, CRM connectors, Windows.

## 4. Packaging & pricing

- **Hobby (free):** local-mode connectors + gallery templates. The privacy architecture is never paywalled — "same privacy standard as Pro" stays true, and free users become the word-of-mouth engine.
- **Pro ($20/mo):** away mode, more usage headroom, extended agent sessions. Away mode is the cleanest Pro reason-to-buy we've ever had: it's a *capability*, not a meter.
- **Positioning vs Town's pricing:** $20 flat against Town's $49 + per-credit overages. Copy line: "no meter running on your own inbox." (Respect existing rule: no ratio claims in copy.)

## 5. Go-to-market

1. **Lead with the disqualifying story, not the feature list.** Profession-led outcome content — the lawyer's morning briefing, the accountant's auto-inbox in busy season — where confidentiality is *why they can finally have an assistant*. Same named-persona, quantified format Town proved, pointed at the audience Town can't touch.
2. **The comparison writes itself.** Anchor line across site, drops, and SEO pages: **"Town's servers can read your inbox and you take their word for it. Clovy's can't — and you don't have to take ours."** New vs-page joins the existing SEO set; comparison rows added to /june. (PR-copy rule still applies: no competitor names in release notes/PR titles — competitor comparisons live on marketing surfaces only.)
3. **Demo the proactive moment.** Launch video is Clovy *volunteering* — noticing repeated work and offering to take it over — because the assistant offering is the shareable clip. Explainer-video pipeline and Fastlane personas carry the b-roll versions.
4. **Open-source as a channel.** The connectors crate and relay are in the MIT repo like everything else; GitHub/HN-native launch for the away-mode threat model specifically — "here is exactly what we can and cannot see, verify it" is HN front-page material and the trust artifact the confidential prosumer's IT-literate friend checks on their behalf.
5. **Sequenced launch:** rc-channel dogfood (capped at 100 users by Google verification anyway) → connectors GA when Google verification clears → away-mode beta with its audit report attached → away-mode GA. Each gate is itself a content moment ("we passed the audit; here's the report").

## 6. Success metrics

*(All billing-derived or aggregate — no per-user app telemetry. This fork's
desktop sends no product telemetry; P3A was removed.)*

| Metric | Target | Why it matters |
|---|---|---|
| WAU connecting ≥1 account within 30 days of GA | ≥40% | Connectors are the product now, not a feature |
| Connector users activating ≥1 gallery routine | ≥25% | Routines are the retention primitive |
| Opt-in weekly active aggregate reporting ≥1 successful routine run | ≥25% | Measures routine adoption with one local weekly boolean and no durable user or cohort identifier |
| Away-mode opt-in among routine users (Pro) | ≥20% | Reason-to-buy working |
| Free→Pro conversion lift post-launch | Directional up | Away mode as the upgrade trigger |
| External audit: plaintext-at-rest findings | Zero | The claim underneath every marketing line |
| Time from install → first routine run | ≤10 min median (design target) | Activation bar |

## 7. Risks & mitigations

| Risk | Severity | Mitigation |
|---|---|---|
| Google restricted-scope verification (CASA) takes 6–12 weeks | **Critical path** | Started day 0; dogfood under the 100-user cap in parallel; GA date keyed to it |
| Prompt injection via email content (assistant reads untrusted input) | High | Approval-default trust modes, earned autonomy, red-team gate before rc; this is also why "the agent can make mistakes" honesty matters commercially |
| Town ships credible zero-knowledge first | High | Speed over polish: local mode alone (Phases 1–2) already beats their privacy story; don't hold connectors GA hostage to away-mode |
| Scope-consent friction kills conversion at the Google screen | Medium | Incremental scopes per feature; readonly first; send-scope only when a send routine is enabled |
| Away-mode complexity delays everything | Medium | Hard phase separation; local mode ships independently and is the majority of value |
| Overclaiming in copy erodes the honesty brand | Medium | Threat-model page is the single source of truth; all claims gate on it (claims-ledger discipline) |

## 8. Decision asks

1. Approve the phased scope (local connectors → templates → relay → Slack/Notion/Linear) and ~12-week plan in the CTO doc.
2. Approve away-mode as Pro-gated; local connectors free.
3. Approve budget for the CASA assessment lab and the external relay security audit (two separate engagements).
4. Green-light the vs-Town marketing surface (vs-page + comparison rows) to build in parallel with Phase 1, gated on the threat-model page for claims.
