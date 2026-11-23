# Design

## Context

Motivação em `proposal.md`. Estado atual, com referências:

- Quadro útil (`src-tauri/src/activity/timeline/builder.rs:254-275`): janela ou texto mudou, ou existe `input_events.occurred_at` em (−3 s, +1 s] do quadro (`input_near`). O ETL carrega esses instantes com margem de 5 s (`timeline/etl.rs:84-99`). Apps `loginwindow` e `screensaverengine` nunca são úteis.
- Entrada capturada: o tap do macOS só escuta mouse down (esquerdo, direito, outro) e key down (`activity/macos/input_tap.rs:26-29`), e só roda com Monitoramento de Entrada (`macos/mod.rs:85-93`). Mouse move, rolagem e trackpad não entram.
- `ActivityPlatform` (`activity/platform.rs:100-132`) não tem método de tempo ocioso do sistema. `frames` não tem coluna para isso (migração 1, `store.rs:290-299`); o catálogo de migrações é append-only e está na versão 5.
- Minutos por atividade: `normalize_minutes(active_minutes, weights)` (`day_intelligence/hour.rs:107`) reparte minutos inteiros por maior resto; `active_minutes = whole_minutes(focused_ms)` (`pipeline.rs:145`, `sources.rs:157-161`). Peso ≤ 0 vira 0. Resultado em `day_hour_reports.activities_json[].minutes` (migração 4). A dobra e a leitura somam esses inteiros (`workstreams.rs:155,174,220`, `db.rs:177-178`). O frontend multiplica por 60 000 (`workstream-timeline-layout.ts:313`) e `formatDuration` já mostra "< 1 min" para 1–59 999 ms (`timeline-utils.ts:60-81`); `DaySummaryPanel.tsx:439-448` usa `"{minutes} min"`.
- As capacidades `activity-capture`, `activity-timeline` e `day-intelligence` só existem como delta em `openspec/changes/add-activity-intelligence/` (40/41).

## Goals / Non-Goals

**Goals:**
- Presença lida do sistema operacional por tick, sem permissão nova e sem guardar o que foi feito.
- Tempos por atividade e por frente com soma exata e sem zero para atividade existente.

**Non-Goals:**
- Mudar o limiar de 5 min das lacunas, a divisão sono/ocioso/pausa ou as categorias.
- Ampliar o tap para mouse move e rolagem (exigiria Monitoramento de Entrada e gravaria eventos que não precisamos).
- Gerar frente para a hora ainda aberta.
- Reprocessar relatórios por hora antigos.

## Decisions

### D1. Tempo ocioso do sistema por tick, gravado no quadro

`ActivityPlatform` ganha `system_idle_ms() -> Option<u64>`. No macOS: `CGEventSourceSecondsSinceLastEventType(kCGEventSourceStateHIDSystemState, kCGAnyInputEventType)`. O motor lê no começo do tick, junto da janela em foco, e grava em `frames.input_idle_ms` (coluna nova, nula quando a plataforma devolve `None`), migração 6.

Alternativas descartadas:
- Ampliar o tap (`kCGEventMouseMoved`, `kCGEventScrollWheel`): depende de Monitoramento de Entrada, que é opcional hoje, e geraria muitas linhas em `input_events`.
- Gravar um evento sintético `presence` em `input_events`: mistura sinal de sistema com eventos do usuário, e o ETL já filtra por janela de tempo, não por tipo.
- Ler `HIDIdleTime` do IOKit: dá o mesmo dado com mais código.

A ausência de exigência de permissão da API é uma premissa a confirmar na tarefa 1.1, antes do resto. Se a API exigir Monitoramento de Entrada, a change para e volta ao dono.

### D2. Regra do quadro útil

Útil também quando `input_idle_ms <= FRAME_SPAN + 1 s` (3 s), ou seja, houve qualquer entrada desde o tick anterior. `StoredFrame` passa a carregar o valor; `push_frame` soma a condição a `window_changed || text_changed || input_near`. Quadro sem o valor segue a regra antiga. Apps de tela bloqueada continuam nunca úteis, mesmo com entrada (digitar a senha no login não é trabalho).

Limiar alternativo descartado: considerar presença por N minutos após a última entrada (ex.: 60 s). Faria leitura parada contar como foco, mas também transformaria saídas curtas em foco e mudaria o significado do limiar de 5 min das lacunas.

### D3. Repartição em segundos com parte mínima

`normalize_minutes` vira `normalize_seconds(active_seconds, weights) -> Vec<i64>`:
1. Peso ≤ 0, não finito ou ausente passa a valer o menor peso positivo da lista (ou 1 se nenhum for positivo).
2. Cada atividade recebe primeiro 1 s, se `active_seconds >= n`; o restante é repartido por maior resto, desempate pelo índice menor.
3. Soma = `active_seconds`.

`active_seconds = focused_ms / 1000` (truncado). A hora continua sem relatório quando o tempo focado arredondado ao minuto é 0, como hoje (`pipeline.rs:146-148`), para não chamar o provedor por segundos soltos.

`HourActivity` ganha `seconds`; `minutes` continua sendo gravado, arredondado, para o prompt da dobra e para leitores antigos. A dobra e a leitura das frentes somam `seconds`, com fallback `minutes * 60` quando o relatório é anterior à mudança. O DTO das frentes passa a expor segundos e o frontend usa `seconds * 1000` em vez de `minutes * 60_000`.

Alternativa descartada: mínimo de 1 min por atividade (como o Meridian). O dono escolheu soma exata com "< 1 min".

### D4. Exibição

Todo tempo de atividade, de frente e o total "ativos" passam por `formatDuration`, que já devolve "< 1 min" para 1–59 s. `DaySummaryPanel` deixa de usar `"{minutes} min"` para atividades. "0 min" só aparece para tempo zero de verdade (ex.: frente vazia, que não deve existir).

## Risks / Trade-offs

- [A API de tempo ocioso exige permissão em alguma versão do macOS] → tarefa 1.1 confirma no app de desenvolvimento sem Monitoramento de Entrada antes de qualquer outra tarefa; se falhar, para e volta ao dono.
- [Mais tempo vira foco e menos vira ocioso; o histórico antigo continua com a regra velha] → aceito; quadros antigos sem o valor não mudam de classificação, então dias passados não "pulam" ao reconstruir.
- [Entrada em outra tela ou app conta como presença no app da frente] → aceito: o quadro já é do app em foco; mexer o mouse sobre outra janela sem trocar o foco é raro e curto.
- [Leitura sem tocar em nada por mais de 5 min continua ociosa] → aceito; é o limite do sinal disponível sem câmera ou heurística de conteúdo.
- [Prompt da dobra ainda usa minutos inteiros] → aceito; o modelo só usa como contexto, os tempos exibidos vêm de `seconds`.

## Migration Plan

- Migração 6 adiciona `frames.input_idle_ms INTEGER NULL`; sem backfill.
- `activities_json` ganha `seconds` nos relatórios novos; leitura tolera ausência.
- Rollback: reverter o código; a coluna nula e o campo extra no JSON são ignorados pela versão anterior.
- Ordem de arquivamento: `add-activity-intelligence` antes desta change, porque esta altera requisitos que só existem naquela delta.
