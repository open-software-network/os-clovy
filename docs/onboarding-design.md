# Clovy Onboarding — Design Doc

**Date:** 2026-06-09
**Inputs:** Wispr Flow onboarding (25-screen capture), Clovy landing privacy copy (`codex/june-landing-privacy-copy` branch of `os-marketing-page`)

---

## 1. What makes Wispr's onboarding world-class (and what we keep)

From the screen capture, Wispr's flow is: **SIGN UP → PERMISSIONS → SET UP → LEARN → PERSONALIZE**, with a persistent stage progress bar. The principles worth stealing:

1. **One ask per screen.** Every screen has exactly one question, one button, or one action.
2. **Show the scary part before asking for it.** Each permission screen pairs the request (left) with a live screencast of the exact macOS dialog/System Settings toggle (right). Users never wonder "what will this look like / where do I click."
3. **Justify every permission in one sentence of plain language.** "Flow will only access the mic when you are actively using it." "This lets Flow put your spoken words in the right textbox."
4. **Close the trust loop.** After permissions: "Thanks for trusting us, we value your privacy" with green checkmarks recapping what was granted.
5. **Verify hardware before teaching.** Mic test ("Do you see purple bars?") and hotkey test ("Does the button turn purple?") are yes/no questions with live visual feedback — failure is caught before the first practice rep, so the magic moment can't whiff.
6. **Learn by doing in simulated apps.** Three practice reps inside fake Slack/Gmail/Notion, escalating in skill: free dictation → scripted self-correction ("Friday at 3, no actually 4" — teaches that fillers and corrections get cleaned up) → whisper mode. Each rep ends with "Good work!"
7. **Immediate, personal reward.** "You just spoke 2.1x faster than the average typist" with the user's real number, then a time-saved projection with an interactive slider.
8. **Personalization should serve the user directly.** Ask only for inputs that immediately improve the onboarding path or product experience.
9. **Viral loop at the exit.** "2 people from your company are using Flow" → create team.
10. **Onboarding doesn't end at the last screen.** First app launch opens a "How would you like to use Flow first?" modal that routes to a first real task.

## 2. Clovy's problem is harder than Wispr's

Wispr teaches **one** behavior (hold fn, talk). Clovy must teach **three** — dictation, meeting notes, agent — plus a privacy story and an honest risk disclosure. Naively copying Wispr's depth for all three features produces a 45-screen onboarding nobody finishes.

### Approaches considered

- **A — Dictation-first spine, staged depth (recommended).** Full Wispr-style hands-on treatment for dictation only (it's the only feature that can deliver a live magic moment in the first 3 minutes — meetings need a meeting, agents need a task worth delegating). Meeting notes and agent get _show-don't-tell_ demos plus the risk/privacy education in onboarding, with heavy permissions and first-runs deferred to just-in-time moments. ~20 screens, ≤7 minutes.
- **B — Three full tracks.** Hands-on practice for all three features up front. Highest comprehension, but 12+ minutes and meeting notes/agent practice would be fake-feeling (no real meeting, no real task). Drop-off risk too high.
- **C — Dictation-only onboarding.** Fastest, but fails the explicit requirements: users must learn meeting notes, the agent, the privacy model, and the agent's risks during onboarding.

**Recommendation: A.** It preserves Wispr's pacing (magic moment by minute 3) while meeting every education requirement. The agent risk disclosure stays _inside_ onboarding (it's a stated requirement), but the agent's file-access permission and first task happen just-in-time, where consent is informed by context.

### Permissions strategy ("same permissions" + Clovy's extras)

| Permission                                          | When asked                       | Why there                                                                                                                                                |
| --------------------------------------------------- | -------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Microphone                                          | Onboarding (Stage 3)             | Core to dictation; same as Wispr. "Clovy only listens while you hold the hotkey or while a meeting note is recording."                                    |
| Accessibility                                       | Onboarding (Stage 3)             | Core to dictation; same as Wispr. "Lets Clovy type your words into whatever app you're using."                                                            |
| Screen & System Audio Recording (for meeting audio) | Just-in-time, first meeting note | Scariest macOS permission; asking during onboarding with no meeting in sight tanks trust and grant-rates. Onboarding _previews_ that this ask is coming. |
| File access for the agent                           | Just-in-time, first agent task   | The agent asks per-scope when given its first task; the onboarding risk screen previews this. Aligns with "nothing changes until you say yes."           |

Onboarding's permission recap screen explicitly sets the expectation: _"Two more permissions will come later, when they make sense: macOS will ask for system audio the first time you record a meeting, and the agent will ask before it touches your files."_

## 3. The flow

Progress bar: **SIGN UP → PRIVACY → PERMISSIONS → SET UP → LEARN → PERSONALIZE**

Layout language is Wispr's: split-screen (task left, illustration/demo right) for signup/permissions; centered card on watermark background for tests and practice; full dark screen for reward moments.

### Stage 1 — Sign up (4 screens)

**1. Welcome.** Logo + "Let's get you started." Sub: "The private AI assistant for your desktop." `Sign in via browser`. Right panel: auto-rotating carousel — dictation typing into a real app ("Works in any app"), meeting notes assembling themselves, the agent finishing a task ("Your work stays on your Mac").

**2. Welcome, {name}!** "What should Clovy help with first?" chips.

**3. Tell us about yourself.** "What do you do for work?" — role chips (feeds later examples: a lawyer sees a contract-summary agent task, a founder sees an investor-update draft).

**4. What should Clovy take off your plate?** Multi-select chips: `Writing by voice` · `Meeting notes` · `Research & drafts` · `Digging through files` · `Recurring busywork`. Copy: "Select all that apply — we'll tailor your setup." This seeds the Learn-stage order and the first agent task suggestions.

### Stage 2 — Privacy (2 screens) — _the Clovy twist_

Wispr buries data-sharing as one screen. Privacy is Clovy's reason to exist, so it gets a named stage — but only two screens, because trust is earned by behavior (the just-in-time asks, the approval gates), not by paragraphs.

**5. "Private by architecture, not by promise."** Three cards (copy adapted from the landing branch):

- **Local by default** — "The agent runs on your Mac. Files, sessions, memory, and state stay on your disk — never mirrored to a cloud."
- **Private inference** — "When Clovy needs a model, your prompt goes out through private routing to zero-retention models by default. Nothing stored. No training on your data. Ever."
- **Verifiable** — "Our code is open source and our backend runs in a secure enclave (TEE). You don't have to trust us — you can check." Link: `Verify it yourself ↗`

Right panel: a simple animated diagram — your Mac (files/memory/agent inside a solid border) with one thin arrow out labeled "prompt → zero-retention model" and nothing coming back but the answer.

**6. Choose your data sharing preference.** Single toggle, **default OFF** (Wispr makes you choose; Clovy's brand demands the private default): "Share anonymized usage analytics to help improve Clovy." Below: "Either way: we store only your account, login, and billing records. Your prompts, transcripts, files, and memory are not on that list. Change anytime in Settings → General." Link: `Learn how we use data`.

### Stage 3 — Permissions (3 screens) — _mirrors Wispr exactly_

**7. Microphone.** Card: "Allow Clovy to use your microphone — Clovy only listens while you hold the hotkey or while a meeting note is recording." `Allow` + ⓘ. Right: screencast of the macOS mic dialog with cursor moving to OK.

**8. Accessibility.** Card: "Allow Clovy to type for you — this lets Clovy put your spoken words into whatever app you're using." Right: screencast of System Settings → Privacy & Security → Accessibility with the Clovy toggle flipping.

**9. Trust recap.** "Thanks for trusting us — here's the full picture." Checkmark cards: `Clovy can use your microphone ✓` `Clovy can type anywhere ✓`. Below, two _preview_ cards (greyed, no checkmark): `System audio — macOS will ask the first time you record a meeting` and `Your files — the agent asks before it touches anything`. This is the screen that converts the permissions stage from extraction into a contract.

### Stage 4 — Set up (3 screens) — _same as Wispr_

**10. Test your microphone.** "Do you see green bars while you speak?" Live level meter. `Change microphone` / `Yes`. Sub: "We recommend built-in or wired microphones — Bluetooth is less reliable."

**11. Set the languages you speak.** "Clovy works in 100+ languages."

**12. Test the hotkey.** "Hold `fn` — does the key light up while you press it?" `Edit shortcut` / `No` / `Yes`.

### Stage 5 — Learn (8 screens) — _dictation hands-on, then notes, then the agent_

**Dictation (4 screens, Wispr's playbook):**

**13. Explainer.** "Clovy starts listening when you hold `fn` — it types what you said when you let go." Animated demo over a real-looking app.

**14. Practice 1 — reply to a message.** Simulated chat app, incoming "Hey {name}, what's up?" Prompt: "Hold `fn`, say something, then release."

**15. Practice 2 — write an email, hands free.** Simulated email. Guided script: _"Umm hi Greg. Let's connect soon. Are you available Friday at 3, no actually 4? Best, {name}."_ The output appears clean — no "umm," the time reads 4. Caption: "Clovy formats for you and fixes your mistakes." (This single rep teaches the entire value prop of AI-cleaned dictation.)

**16. Reward.** Dark screen: "Nice job! You just spoke **{n}× faster** than the average typist." Then the time-saved slider: "With Clovy you could save **{x} hours a week**."

**Meeting notes (2 screens, show-don't-tell):**

**17. Demo.** "Never take notes again." A 20-second simulated meeting plays (two voices, audio optional) while a notes panel assembles itself live: **Decisions**, **Action items (with owners)**, **Who said what**. Footer: "Transcripts and notes stay on your Mac." This is watched, not practiced — there's no real meeting to practice on.

**18. Choice.** "How should meeting notes start?" Two cards: **Detect my meetings** ("Clovy notices when a meeting starts and offers to take notes — you always see a recording indicator") / **I'll start them manually** (menu-bar button + hotkey). Either way: "The first time, macOS will ask for system-audio access — that's the permission we mentioned."

**Agent (2 screens — intro + the honesty screen):**

**19. Intro.** "Hand off real work." Copy: "Give Clovy a task, not just a question. Draft the doc, dig through the files, pull the research together — the agent works on your Mac and comes back with it done." Right panel: the approval-card preview from the landing page — _"Clovy found the file and prepared the edit. Nothing changes until you say yes."_ `Approve` / `Decline`.

**20. The honesty screen.** (Full copy in §4 below.) States plainly that the agent can make mistakes, explains what guardrails exist, and draws the line between private inference and the consequences of agent actions. Ends with an explicit acknowledgment checkbox — a seatbelt moment, not a EULA. This screen is _not skippable_; it gates the agent feature, not the app.

### Stage 6 — Personalize & finish (3 screens)

**21. First agent task (optional, seeded by screen 4).** "Want to hand Clovy its first task?" Three suggestion chips matched to their selections, all read-only-safe: "Summarize everything in a folder I pick" · "Research a topic and draft a one-pager" · "Prep a brief for my next meeting." Choosing one drops the user into the real agent UI with the approval pattern visible. `Skip for now` is prominent — this can also be the post-onboarding modal's job.

**22. Team.** "{n} people from your company are using Clovy" → create team / invite. (Privacy-conscious: shown only from same-domain signups, with the same opt-out Wispr offers.)

**23. Done → main app**, which opens the "What would you like Clovy to do first?" modal: `Write something by voice` · `Take notes in my next meeting` · `Give the agent a task`, each deep-linking into the real feature.

## 4. The two copy moments that carry the requirements

### Screen 5 — privacy education (adapted from landing copy)

> **Private by architecture, not by promise.**
> Every layer of Clovy defaults to private. The ones that matter most, you can verify.
>
> **Local by default.** Clovy owns the agent runtime on your Mac. Your files, sessions, and memory stay on your disk.
> **Private inference.** Prompts leave your Mac only for model inference, routed by default to zero-retention models — nothing stored, nothing trained on. Third-party models are opt-in, with your identity stripped.
> **Verifiable.** Open-source code, TEE-attested backend. Check it yourself — don't take our word for it.

### Screen 20 — the honesty screen (agent risks + the inference/agent distinction)

> **Before you meet the agent, three honest things.**
>
> **1. The agent can make mistakes.**
> It's powerful, and that means it can misread a file, take a wrong step, or sound confident while being wrong. Treat its work like a sharp new hire's: useful, fast, and worth a glance before it ships.
>
> **2. So nothing irreversible happens without you.**
> The agent asks before it edits or deletes files, sends anything, or spends anything. Every session has a full activity log, and you can stop it at any moment. Nothing changes until you say yes.
>
> **3. Private inference protects your data — it doesn't approve the agent's actions.**
> When Clovy _thinks_, your prompts go to zero-retention models: nothing stored, nothing trained on. That's the privacy of inference, and it's always on.
> When the agent _acts_ — visits a website, calls a tool you've connected, sends an email you approved — the other side sees what it shares, exactly as if you'd done it yourself. Clovy keeps your data private; it can't make the rest of the internet private. That's why you're the approval step.
>
> ☐ I understand the agent can make mistakes, and I stay in control of what it does.
> `Meet the agent`

The structural insight this screen encodes: **inference privacy is a property of Clovy (always on, verifiable); action risk is a property of what you authorize the agent to do (scoped, approved, logged).** Two different layers, two different mitigations — and the user leaves onboarding able to articulate both.

## 5. Success metrics

- **Activation:** % completing first dictation rep (target: match Wispr-class ~85%+ of permission-granters); % reaching the reward screen.
- **Per-stage drop-off**, especially screens 5–6 (does the privacy stage cost completion?) and screen 20 (does the honesty screen scare people off the agent — or increase first-task rates, as informed consent usually does?).
- **Permission grant rates** (mic, accessibility at onboarding; system audio + file scope at just-in-time) — JIT asks should outperform 80%+.
- **Feature adoption at day 7:** dictated ≥3 days, ≥1 meeting note, ≥1 agent task.
- **Comprehension probe (qualitative/UXR):** can users answer "what does Clovy store about you?" and "what's the difference between Clovy's privacy and the agent emailing someone?"

## 6. Open questions

1. **Hotkey default** — assumed `fn` like Wispr; does Clovy have its own (and is fn already claimed by Wispr on machines running both)?
2. **Meeting-notes demo audio** — real recorded scene vs. silent animated transcript? (Audio is far more convincing; needs production.)
3. **Screen 21 first-agent-task** — in onboarding (recommended, optional) or fully deferred to the post-onboarding modal?
4. **Time-saved projection numbers** — need real WPM/typing-time model before shipping the reward screen claims.
5. **Team detection** — does the same-domain lookup conflict with the privacy positioning? (Recommend: only surface counts, never names, until invited.)

---

## 7. No-card onboarding and funding (2026-06-24)

> Superseded in this fork by §8: sign-in, funding gates and billing were
> removed.

The onboarding path should get a signed-in user into the product before any
payment handoff. OS Accounts owns the starting grant, so Clovy should trust the
account snapshot instead of minting trials or opening checkout from the
desktop app.

### Current funnel

Progress bar: **Sign in -> Privacy -> Focus -> Mood -> Permissions**

1. **Sign-in stays inside onboarding.** The first step sends the user through
   OS Accounts and returns via the `osjune://` deep link. After the account
   snapshot refreshes, the wizard continues without asking for card details.
2. **Personalization stays lightweight.** One focus choice and one mood choice
   shape Clovy without turning setup into a survey.
3. **Permissions finish onboarding.** Microphone, accessibility, and
   platform-specific setup are ready before the user enters the product. The
   first real action happens in Clovy itself instead of a simulated practice
   screen. Onboarding never opens Stripe, the OS Accounts portal, or a trial
   checkout.

### Post-onboarding gates

`AccountGate` blocks signed-out users. Funding never blocks the shell: for
accounts that are signed in, have a known zero credit balance, and do not have
a live subscription, only credit-consuming actions are gated, and a persistent
`FundingNotice` (above the chat composers, plus the sidebar `FundingChip`)
offers the way out. Unknown credit snapshots are allowed through so the metered
actions can make the final decision. Past-due billing opens the OS Accounts
portal; zero-credit accounts open the same portal to add credits.

### Server dependencies

- OS Accounts should grant the initial credits for new users server-side.
- The Clovy OAuth scopes should stay read/spend focused:
  `profile:read billing:read credits:spend`. The desktop app should not need
  `billing:write` because billing changes happen in OS Accounts.

## 8. Account-free onboarding (fork, 2026-10-21)

The `desvincular-servicos-open-software` change removed OS Accounts from the
desktop. The app runs on a fixed local identity, so there is nothing to sign in
to and nothing to fund.

Progress bar: **Focus -> Mood -> Permissions** (`ONBOARDING_STEPS` in
`src/components/onboarding/OnboardingFlow.tsx`).

- There is no sign-in step, no `osjune://` deep link, and no telemetry consent
  step; the data sharing screen in §3 and the sign-in step in §7 are gone.
- After onboarding the app opens the main window directly. There is no
  `AccountGate`, `FundingNotice` or `FundingChip`, and no action is gated on a
  plan or credit balance in the desktop.
