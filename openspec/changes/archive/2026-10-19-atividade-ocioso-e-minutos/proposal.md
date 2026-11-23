# Proposal

## Why

A tela "Hoje" mostra dois números que enganam. Ler um documento rolando a página ou só mexendo o mouse vira "Ocioso", porque o Clovy só conta clique e tecla como presença. E uma atividade real da hora pode aparecer como "0 min" numa frente de trabalho, porque os minutos medidos são repartidos em minutos inteiros e quem fica sem resto recebe zero.

## What Changes

- A captura passa a gravar, em cada quadro, há quanto tempo o sistema não recebe nenhuma entrada (teclado, mouse, rolagem, trackpad), lido do próprio sistema operacional a cada tick. Não grava o tipo nem o conteúdo da entrada.
- Um quadro sem mudança de janela ou texto passa a contar como útil quando houve qualquer entrada do sistema desde o tick anterior. Mouse e rolagem deixam de virar "Ocioso". Sem esse dado (sistema não informa), vale a regra atual.
- A repartição do tempo ativo da hora entre as atividades passa a ser feita em segundos. A soma continua igual ao tempo medido, e toda atividade listada pelo modelo recebe uma parte positiva.
- Atividades, frentes e o cabeçalho "N ativos" mostram "< 1 min" para partes abaixo de um minuto, nunca "0 min" para uma atividade que existiu.
- Relatórios por hora já gravados (só com minutos inteiros) continuam legíveis.

## Capabilities

### New Capabilities

Nenhuma.

### Modified Capabilities

- `activity-capture`: cada quadro passa a registrar o tempo desde a última entrada do sistema.
- `activity-timeline`: a regra de quadro útil passa a considerar a entrada do sistema; o que é quadro útil vira requisito explícito.
- `day-intelligence`: os tempos por atividade passam a ser repartidos em segundos, com parte positiva para toda atividade e exibição "< 1 min".

Essas três capacidades ainda vivem só na change `add-activity-intelligence`, não em `openspec/specs/`. Ela precisa ser arquivada antes desta.

## Impact

- Rust: `src-tauri/src/activity/platform.rs` (novo método de tempo ocioso do sistema), `activity/macos/` (leitura do tempo ocioso), `activity/engine.rs`, `activity/store.rs` (migração 6 com coluna em `frames`), `activity/timeline/etl.rs` e `timeline/builder.rs` (regra de quadro útil), `src-tauri/src/day_intelligence/hour.rs`, `pipeline.rs`, `workstreams.rs`, `db.rs` (segundos por atividade e soma das frentes).
- Frontend: `src/components/activity-timeline/WorkstreamTimeline.tsx`, `workstream-timeline-layout.ts`, `timeline-utils.ts`, `DaySummaryPanel.tsx`, mensagens em `src/i18n/messages/dayIntelligence.ts`.
- Docs: `docs/activity-capture.md`, `docs/activity-timeline.md`, `docs/day-intelligence.md`.
- Dados: `activities_json` ganha segundos por atividade; relatórios antigos seguem com minutos. Sem mudança de provedor, permissão nova ou dependência nova.
- A change `atividade-windows` depende do novo método da plataforma e implementa o equivalente no Windows.
