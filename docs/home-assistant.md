# Home conversation

> Registro histórico. A interface Início foi retirada em
> [ADR-0075](adr/0075-desktop-opens-today-and-approves-external-mcp-actions.md).
> Hoje recebe a navegação antiga. Os dados descritos abaixo permanecem gravados.

Home is Clovy's persistent conversation on the home screen. Since
[ADR-0064](adr/0064-home-runs-on-the-default-cli-engine.md) every Home message
is a turn of the default CLI engine, in a hidden session, without a remote chat route.

## Product contract

- Each data partition has at most one stored Home session id on this device
  (`clovy:home:session-ids:v1`).
- Home never appears as an item in the Sessions list or the native
  recent-session shortcuts.
- A Home message is `start_agent_run` on the Home session with the default
  chat engine (`chat_default_engine`: Claude Code, else Codex) in `sandboxed`
  mode, through the same `start_cli_run` → `start_cli_turn` path as any CLI
  session ([llm-providers.md](llm-providers.md#cli-chat-engine)). Home has no
  model, engine, or safety mode picker. With neither CLI installed the turn
  asks Claude Code, so it fails with the CLI sessions' missing-engine error.
- The Home session is created on the default engine at the first send. A Home
  session from an older build (`open-software/auto`) adopts the engine with its
  next turn, because `start_agent_run` stores the turn's model on the session.
- The conversation is the Home session's items, rendered with the same
  projection as a CLI session: streaming, approval and clarification cards,
  errors with retry, and the unsent-message recovery row. Continuity comes from
  the CLI's `--resume`, or the prefixed history when there is no CLI
  conversation to resume.
- Home answers and works in place. It does not classify a message, force
  research, or open a separate session. It carries no Clovy persona or fixed
  identity reply: a Home turn is the same as any CLI turn.
- Home keeps lightweight suggestion nudges above the composer. They prefill the
  empty conversation, follow the local time of day, and never run work without
  the user's Send.

## History from older builds

Older builds kept the Home conversation only in the webview's local storage
(`clovy:home:direct-turns:v1`) and its task cards in
`clovy:home:task-handoffs:v1`.

- On open, Home sends the stored turns to `import_legacy_home_turns`, which
  writes their text as user and assistant items of the Home session, in order
  and at their original time, merged by time with the items already there.
  Unreadable entries are skipped and counted; turns with no text (a card's tool
  turn, an attachment-only message) are not imported.
- The import happens once: every imported item carries the external id
  `home-import:<stored session id>:<turn id>`, and any such item marks the
  session as imported. Home may ask on every open; later calls import nothing.
- Imported turns reach the CLI as prefixed history on the next turn.
- Local storage is never cleared, so a rollback still finds the old thread.
- The old task cards keep rendering from local storage, placed by the time of
  the stored turn that carried them, and open the focused session they started.
  A card left starting was interrupted and shows as failed. Nothing creates new
  cards.
