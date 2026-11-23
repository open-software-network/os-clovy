# Issue tracker: harness tickets

Work for this fork is tracked as tickets in the owner's harness
(`~/orchestrator`), not in GitHub Issues. A ticket names the change; the
OpenSpec change under `openspec/changes/<name>/` holds what to build and its
tasks.

## Conventions

- Branches use the ticket or change name (for example `feat/<change>`), not a
  `JUN-*` prefix.
- Read the ticket brief and the OpenSpec change before writing code; mark
  tasks in the change's `tasks.md` with short evidence as they finish.
- Commit subjects describe the change in English in the repo's style; reports
  for the owner are in pt-BR.

## When a skill says "publish to the issue tracker"

Write the result into the ticket's report (for example
`.tmp/verify/<ticket>/report.md`) and let the harness carry it. Do not open
GitHub Issues unless the owner asks.
