# Design

## Context

- `dictation_history` tem `id, text, language, provider, profile, created_at`; não guarda app. A linha é escrita por dois caminhos em `transcribe_recording_ready_inner` (`dictation.rs`): fala curta (`stage_dictation_history_item`, antes da entrega) e normal (`deliver_dictation_outcome` → `spawn_dictation_history_write` → `persist_dictation_history_item`). Nos dois, `recording.target_bundle_id` (fixado pelo helper no `recording_ready`) já está disponível.
- O HUD resolve ícone pelo comando `activity_app_icon(bundleId)` (PNG 64×64 em data URL, cache no Rust e no TS). O componente `AppIcon` (`src/components/activity-timeline/AppIcon.tsx`) já usa esse comando.
- O pipeline `day_intelligence` monta prompts a partir de structs com seções opcionais (reuniões, blocos de agentes) dentro de `fence()`. Fontes externas entram pelo trait `DaySources` (`sources.rs`), implementado por `AppSources` (banco principal), `McpDaySources` e fakes de teste.

## Goals / Non-Goals

**Goals:**
- Um único campo novo (`app_bundle_id`) alimenta o ícone do histórico e o app no contexto do modelo.

**Non-Goals:**
- Mudar os prompts de sistema, os schemas JSON, os gates de "sem atividade", a dobra de frentes de trabalho ou os painéis do Hoje.
- Regenerar relatórios já gravados.
- Backfill de ditados antigos ou bundle id no Windows.
- Guardar nome de exibição do app no histórico.

## Decisions

1. **Coluna `app_bundle_id TEXT` anulável, migração 54 com `EnsureColumns`** (padrão da migração 22). Sem backfill; inserts existentes em testes seguem válidos. Fixar o valor ao lado de `history_profile` em `transcribe_recording_ready_inner` e passá-lo pelos dois caminhos de escrita. Bundle id vazio vira `NULL`.
2. **Histórico reaproveita `AppIcon`**: com `appBundleId`, o chip mostra o ícone do app; sem ele, o microfone. Para cair no microfone também quando o ícone não existe, `AppIcon` ganha uma prop opcional de fallback (padrão atual inalterado para o timeline). O diálogo do item usa o mesmo ícone. Nenhuma biblioteca de ícones nova.
3. **Ditados entram como dados, não como ferramenta.** O pipeline faz chamadas únicas com contexto montado; uma ferramenta de consulta exigiria um agente com tools. Novo método `DaySources::dictations(from, to, zone)` com corpo padrão vazio, sobrescrito só em `AppSources`; `McpDaySources` e fakes compilam sem mudança. Consulta `dictations_between` filtrando `profile` e `created_at` por `julianday`, como `meetings_between`; erro vira `warn` e lista vazia.
4. **Seção só quando não vazia**: em `hour::prompt` depois das reuniões (`"\nDictations (what the user said aloud, with target app when known):\n"`) e em `summary::prompt` como `=== DICTATIONS ===`, no mesmo molde da seção de agentes de código. Lista vazia mantém o prompt byte a byte.
5. **Gates intocados**: ditados não entram no gate de minutos ativos da hora nem no gate `reports/meetings/blocks` do resumo.
6. **Nome do app**: no relatório por hora, resolver `bundle_id → app_name` pelas sessões do timeline já carregadas na hora; no resumo do dia, pelas sessões do dia se já disponíveis, senão o bundle id. Sem bundle id, a linha sai sem app.
7. **Limites**: por hora, até 20 ditados de até 400 caracteres (`sources::excerpt`); no dia, até 40 ditados de até 240 caracteres. Corte por `chars()`. Acima do limite, ficam os primeiros em ordem cronológica e uma linha final `(+N more)`. Alternativa rejeitada: amostrar ao longo do período, mais complexo sem ganho claro.

## Risks / Trade-offs

- [Texto falado vai ao provedor de atividade, que pode ser diferente do provedor de ditado] → mesma cerca e regras de dados das notas de reunião; documentar em `docs/day-intelligence.md`.
- [Retenção de 7 dias do histórico] → só relatórios dos últimos 7 dias ganham o contexto; relatórios são gerados na hora seguinte, então não há perda prática.
- [Destino fixado pode diferir do app em primeiro plano] → é o app onde o texto foi colado, que é o dado mais útil.
- [Testes que fixam a lista de migrações] → atualizar as asserções de versão em `migrations.rs`.
