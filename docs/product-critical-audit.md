# Product critical audit

Date: 2026-06-17

## Thesis

June has the right raw ingredients: desktop capture, dictation, meeting notes,
local files, a local agent runtime, routines, and a strong privacy story. The
problem is that those ingredients currently read as several useful utilities
next to each other, not one product users need.

The product should collapse around a sharper promise:

> Speak once. June turns it into the right work artifact, in the place you
> already work, without giving up control of your private context.

That promise can unify dictation, meeting notes, and the agent. Dictation is the
habit wedge. Meeting notes are the context capture layer. The agent is the
follow-through layer. Privacy is the permission to trust the whole system, not
the core job by itself.

## What users will actually care about

Users will not care that June has a model picker, a TEE, routines, projects, or
multiple capture modes until those things support a felt outcome. The strongest
user concerns are more basic:

1. Does this save me time today?
2. Does it work in my real apps without breaking my flow?
3. Is it fast enough to become muscle memory?
4. Does it remember my words, names, format, and writing style?
5. Does it make me look prepared and responsive?
6. Can I trust it with meetings, files, and private work?
7. When it fails, do I understand what happened and can I recover?

The first four are adoption. The last three are retention.

## Market read

The competitive pattern is clear: products that feel necessary pick a tight
daily habit, then expand from it.

- [Wispr Flow](https://wisprflow.ai/) leads with dictation in every app, speed,
  AI cleanup, and broad device support.
- [Superwhisper](https://superwhisper.com/) also leads with voice-to-text in any
  app, then expands into local/cloud models and custom modes.
- [Granola](https://www.granola.ai/) leads with meeting notes that do not invite
  a meeting bot, then monetizes history, AI chat, templates, and sharing in
  [pricing](https://www.granola.ai/pricing).
- [Otter](https://otter.ai/) has moved from transcription toward meeting
  agents, action items, integrations, and cross-meeting workflow.
- [Plaud](https://www.plaud.ai/) and the
  [Limitless Pendant](https://www.limitless.ai/new) make capture physical and
  ambient. Their bet is that remembering conversations is valuable enough to
  justify dedicated hardware.
- [Raycast AI](https://www.raycast.com/core-features/ai) owns the OS shortcut
  and command habit, then uses AI commands as the automation layer.
- [ChatGPT Record](https://help.openai.com/en/articles/11487532-chatgpt-record)
  makes meeting transcription and summaries available inside an already-paid
  general AI subscription.

This means June is not competing with one category. It is competing with several
clear habits:

- "I press a key and speak instead of typing."
- "I stop taking notes in meetings."
- "I press a shortcut and the OS does a task."
- "Everything I say becomes searchable memory."

June can still win, but only if the combination is presented as one compounding
workflow instead of a feature menu.

## Evidence from the current product

The README positions June as "meeting notes, dictation, and agent work" at equal
weight. That is accurate, but it is too horizontal for first-use activation.

`docs/onboarding-design.md` already identifies the right direction:
dictation-first, with meeting notes and agent demos layered in after the live
magic moment. The shipped flow in
`src/components/onboarding/OnboardingFlow.tsx` is much thinner: sign-in,
permissions, trial, and one dictation practice step on macOS.

`src/components/onboarding/steps/PracticeStep.tsx` verifies that words arrive,
but it does not prove the full value loop:

- It accepts a four-character success condition.
- It can be skipped.
- It asks "What should we work on first?" but does not route the answer into an
  actual first task.
- It does not show cleanup, style, correction handling, or time saved.

`src/components/account/TrialGate.tsx` makes the product unusable without an
active trial or subscription. That can be correct for billing, but it raises the
standard for activation: the trial gate must be followed immediately by a
convincing win. Today the user is asked to trust the product before the product
has demonstrated enough.

`src/components/agent/AgentWorkspace.tsx` has capable agent infrastructure, but
the first impression is generic. The hero asks "What can June do for you?" and
rotates broad suggestions like research, file lookup, computer health, and
screenshot search. Those are useful, but they do not yet communicate a must-have
daily workflow.

`src/components/notes-list/NotesList.tsx` sells meeting capture with generic
"transcribes it and writes the note for you" copy. The stronger job is not the
note. It is being able to listen in the meeting, then send the follow-up, extract
decisions, assign owners, remember context, and prepare for the next meeting.

`src/components/dictation/DictationHistoryView.tsx` has a sensible empty state
and a style/dictionary hint after usage, but the product waits too long to teach
why style and dictionary matter. If users dictate names, project terms, and
commands early, this becomes personal faster.

`src/components/settings/ModelPickerDialog.tsx` and the Models settings are
powerful, but they expose infrastructure complexity earlier than most users
need it. Model choice is an expert affordance. The mainstream affordance should
be "fast", "private", or "best quality", with sane defaults.

`src/components/routines/routine-templates.ts` is one of the most strategically
interesting areas because routines can create recurring dependency. But routines
are currently framed as an advanced feature instead of a retention loop:
"June already prepared the things you would otherwise have to gather yourself."

## What is going wrong

### 1. The product is too broad at the top

June currently asks users to understand meeting notes, dictation, agent work,
projects, routines, models, privacy, billing, and permissions. That is too much
before a user has a habit.

The fix is not to remove capability. The fix is to sequence capability behind a
single initial job: speak and get useful work back.

### 2. Onboarding does not deliver a durable aha moment

The design doc is much better than the implementation. The shipped onboarding
proves that the microphone and shortcut work, but not that June is meaningfully
better than built-in dictation.

The practice should prove at least one of these:

- June cleans up rambling speech into usable writing.
- June formats for the destination app.
- June handles corrections like "no, actually".
- June remembers custom names and terms.
- June turns spoken intent into an artifact or next action.

Without that, the user has not learned why June should replace an existing
habit.

### 3. The trial gate comes before enough conviction

June can require a trial because metered inference costs money. But the current
experience makes the trial feel like a hurdle before payoff. After checkout,
the app needs a high-confidence win within seconds, not a generic product
surface.

The trial pitch should be tied to the immediate action it unlocks: "Start the
real dictation practice" or "Record your first meeting note", not "Try
everything June can do."

### 4. The agent is framed as a general assistant

"What can June do for you?" is a weak first prompt because it puts product
strategy work on the user. People do not need a generic local agent. They need
specific work done:

- "Write the follow-up from this meeting."
- "Turn this messy thought into a polished Slack update."
- "Find the screenshot I took yesterday."
- "Prepare me for my next call from recent notes."
- "Draft the issue from this bug report."
- "Clean up the folder, but ask before changing anything."

The agent should feel like the completion step for captured context, not a
separate chat product.

### 5. Meeting notes stop too early

Meeting notes are crowded. A transcript plus generated note is no longer enough.
The unmet pain is after the meeting:

- What decisions were made?
- Who owns what?
- What should I send?
- What changed since the last meeting?
- What should I prepare before the next one?
- Which open loops are now stale?

June's local memory and agent runtime are exactly the pieces that can make this
stronger than a note taker. The product should sell the follow-through, not the
capture.

### 6. Privacy is differentiated, but not sufficient

Privacy removes objections. It does not create a daily habit by itself.

The strongest use of privacy is contextual: at the moment June asks for the mic,
system audio, file access, or an agent approval, the product should show exactly
what stays local, what leaves, and what action is being authorized.

Privacy controversies around AI meeting products, like
[Granola link-sharing coverage in The Verge](https://www.theverge.com/ai-artificial-intelligence/906253/granola-note-links-ai-training-psa),
create an opening for June. But the opportunity is operational trust, not more
abstract privacy copy.

### 7. Model and credit concepts leak into the product

Model choice, provider privacy, token pricing, credits, and billing are real
system concerns. Most users experience them as anxiety unless they are hidden
behind clear outcomes.

The default experience should pick the right model automatically. The visible
choices should map to user priorities:

- Fastest
- Best quality
- Most private
- Advanced

Credits should never surprise users in the middle of a core workflow. If a
workflow cannot run, the user needs a visible, local, actionable explanation
before they lose trust.

## What could be better

### Make dictation the daily habit wedge

Dictation has the highest frequency, the fastest feedback loop, and the lowest
coordination dependency. It can become muscle memory if latency and paste
reliability are excellent.

Targets:

- First session: three successful dictations, not one.
- First day: ten successful dictations.
- D7: dictated on at least three different days.
- Latency: instrument p50 and p95 from shortcut release to paste.
- Reliability: instrument paste success, cleanup success, and fallback copy.

Product changes:

- Make onboarding practice show cleanup on a realistic messy sentence.
- Add a real time-saved reward after practice.
- Teach correction handling explicitly.
- Ask for custom names and project vocabulary earlier, or infer them from
  accepted corrections.
- Offer destination-aware modes: Slack, email, doc, prompt, task.
- Keep the history, but make retry/copy/failure recovery obvious.

### Reframe meeting notes as meeting follow-through

The core promise should be: "June helps you leave the meeting with the next
work already done."

Product changes:

- Generate a follow-up draft by default.
- Extract decisions, action items, owners, deadlines, and unresolved questions.
- Add "prepare for next meeting" from prior notes and open loops.
- Add one-click exports to the places users already use.
- Let users ask across meetings: "What did we decide about pricing?"
- Use projects as living context, not just folders.

### Make the agent task-specific and context-native

The agent should stop asking users to imagine possibilities. It should offer
workflows attached to context June already has.

Better first-run suggestions:

- From a meeting note: "Send the follow-up", "Create tasks", "Prep next call".
- From dictation history: "Turn this into an email", "Turn this into a PRD",
  "Make it shorter".
- From a folder: "Summarize what changed this week", "Find the contract",
  "Draft a status update".
- From routines: "Every morning, brief me on open loops."

The agent wins when the user feels, "June knows enough about my work to finish
the next step."

### Move model choice down, move outcomes up

Keep model control for power users, but reduce its prominence in the main
workflow. "Kimi K2.6" or "GLM" is not a product promise. "Fast private answers"
is.

Recommended structure:

- Default: June chooses.
- Simple modes: Fast, Best, Private.
- Advanced: model catalog, pricing, privacy badge, tool support.

### Turn routines into retention

Routines are strategically important because they create recurring value without
the user remembering to open June.

Promote routines after June has captured enough context:

- "Want this every morning?"
- "Want June to watch this topic?"
- "Want a weekly open-loops review?"
- "Want Downloads tidied every Friday with approval?"

Do not sell routines as cron for agents. Sell them as June showing up with work
already organized.

## Recommended product direction

### Positioning

Use this as the internal positioning:

> June is the private voice-to-work layer for your desktop.

External copy can be more concrete:

> Talk naturally. June writes, remembers, and follows through in your apps.

Avoid leading with "AI assistant". That category is too broad and already
owned by larger products. Lead with the behavior change.

### Product hierarchy

1. Dictation: the daily habit.
2. Meeting notes: the context capture layer.
3. Follow-through: agent actions generated from captured context.
4. Routines: recurring dependency.
5. Model choice: advanced control.

### Activation loop

The first-run loop should be:

1. Sign in.
2. Grant mic and accessibility with clear rationale.
3. Start trial exactly when the real metered practice begins.
4. Complete three dictation reps:
   - free sentence
   - messy sentence cleaned up
   - destination-specific output
5. Show speed/time saved.
6. Ask what they want June to do next:
   - dictate in any app
   - record the next meeting
   - turn a spoken thought into a document
7. Route directly into that action.

### Retention loop

The D7 loop should be:

1. User dictates daily.
2. June learns names, style, and vocabulary.
3. User records meetings.
4. June generates follow-ups and open loops.
5. Routines package those open loops into recurring briefs.
6. The agent completes the next artifact from captured context.

That is how June becomes a system, not a collection of tools.

## Metrics to add or elevate

Activation:

- Time to first successful dictation.
- Onboarding permission grant rate.
- Trial checkout completion rate.
- Dictation practice completion rate.
- Practice skip rate.
- First-session dictation count.

Habit:

- Dictations per activated user on D1, D7, and D30.
- Dictation usage days per week.
- Shortcut release to paste p50 and p95.
- Paste success rate.
- Cleanup success rate.
- Manual correction rate after paste, if measurable.

Meetings:

- Recording started to note generated success rate.
- Recording retry rate.
- Note opened after generation.
- Follow-up draft copied/exported/sent.
- Action items accepted or edited.
- Cross-meeting search usage.

Agent:

- First agent task success.
- Task started from captured context vs blank hero.
- Approval card accept/decline rate.
- Artifact opened/downloaded.
- Task retry/failure rate.

Billing and trust:

- Insufficient-credit interruption rate by workflow.
- Top-up conversion from interruption.
- Permission denial and recovery.
- Privacy/settings page visits before activation.

## 30/60/90 day roadmap

### Next 30 days

- Tighten onboarding around three dictation reps and a visible reward.
- Rewrite the trial step to unlock the immediate practice, not the whole app.
- Replace generic agent hero suggestions with context-specific artifact
  workflows.
- Instrument dictation latency, paste success, cleanup success, and practice
  skip rate.
- Move model catalog language toward advanced settings and simplify visible
  choices.

### Next 60 days

- Make meeting notes produce follow-up drafts, action items, decisions, and
  unresolved questions by default.
- Add "prep next meeting" from prior notes and project context.
- Add destination-aware dictation modes for email, Slack, docs, and tasks.
- Promote dictionary and style setup earlier based on actual dictation usage.
- Create a post-meeting agent entry point: "Finish the follow-up."

### Next 90 days

- Promote routines as recurring briefs once users have enough captured context.
- Build persona-specific templates for founders, engineers, sales, recruiting,
  legal, and operators.
- Add integrations only where they complete follow-through: calendar, tasks,
  docs, Slack/email export.
- Build cross-meeting and cross-project memory around decisions and open loops.
- Package the privacy architecture into contextual trust moments at sensitive
  actions.

## Suggested follow-up PRs

1. Implement three-rep dictation onboarding with cleanup and time-saved reward.
2. Replace the trial pitch with an "unlock real practice" step.
3. Add activation and habit analytics events listed above.
4. Redesign the agent hero around context-specific artifact workflows.
5. Add meeting follow-up output to note generation.
6. Introduce simple model modes and move full catalog controls under Advanced.
7. Promote routines after sufficient context exists.

## Main risk

If June keeps presenting itself as a private meeting note app plus dictation plus
an agent, it will be compared feature-by-feature against sharper products in
each category. That is a bad comparison.

June's advantage is the loop across categories:

- Speak into any app.
- Capture the meeting.
- Remember the context.
- Finish the follow-up.
- Repeat automatically.

Make that loop obvious, fast, and reliable. That is the path from nice-to-have
to need.
