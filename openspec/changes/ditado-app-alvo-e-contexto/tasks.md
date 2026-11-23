# Tasks

## 1. App de destino no histórico (dados)

- [x] 1.1 Migração 54 com `EnsureColumns` adicionando `dictation_history.app_bundle_id TEXT` anulável; atualizar as asserções de versão em `migrations.rs`; verificar com `cargo test migrations`.
- [x] 1.2 Passar `app_bundle_id` (fixado de `recording.target_bundle_id` ao lado de `history_profile`, vazio vira `NULL`) pelos dois caminhos de escrita e pelos repositórios; expor em `DictationHistoryItemDto` (Rust e `src/lib/tauri.ts`); verificar com testes em `tests/storage.rs`/`tests/dictionary.rs` e um teste de que o valor gravado é o do fim da gravação.

## 2. Ícone no histórico (UI)

- [x] 2.1 Em `DictationHistoryView`, usar `AppIcon` com `appBundleId` na linha e no diálogo, com fallback para `IconMicrophoneSparkle` sem app ou sem ícone (prop opcional de fallback em `AppIcon`, padrão inalterado); ajustar CSS do chip; verificar com testes em `src/test/dictation-history.test.tsx` (com app, sem app, falha do ícone) e `activity-timeline.test.tsx` intacto.

## 3. Ditados no contexto do day intelligence

- [x] 3.1 Adicionar `DictationNote` com `prompt_line()`, `dictations_between` e `DaySources::dictations` com corpo padrão vazio, sobrescrito em `AppSources`; verificar com teste da consulta (perfil, janela de tempo, ordem).
- [x] 3.2 Incluir a seção de ditados em `hour::prompt` e `summary::prompt`, com limites e resolução do nome do app; buscar os ditados em `report_hour` e `generate_summary` sem tocar nos gates; verificar com os testes dos cenários do spec (hora com ditado, lista vazia byte a byte, ditado sem atividade, dia só com ditados, cerca escapada, limites) e `cargo test day_intelligence` verde.
- [x] 3.3 Documentar em `docs/day-intelligence.md` a nova fonte e a linha de privacidade; verificar que o texto bate com o comportamento.

## 4. Integração

- [x] 4.1 Rodar `pnpm check`, `pnpm typecheck`, `pnpm test`, `cargo fmt --check`, `cargo clippy -D warnings` e `pnpm test:rust`; todos verdes. No Mac, comandos nativos sem a fila global removida; na box, preservar os locks Macro→Clovy.
- [ ] 4.2 No app de desenvolvimento: ditar em dois apps diferentes e conferir os ícones no histórico com Computer Use; gerar um relatório de hora com provedor falso ou real e conferir no log/prompt a seção de ditados; registrar evidência no relatório da entrega.

## Workflow follow-up

- Gate `review-pair.sh --gate` antes do commit final.
- Arquivar a change, `/sync` e `/cleanup`.

## Prova executada em 2026-11-03

Biblioteca nativa: 2.302 aprovados/16 ignorados na rodada 8 cobre migrations, os dois caminhos de destino e consulta/prompts/gates de day intelligence; storage (15 testes) na rodada 12 e dictionary (três testes) na rodada 8. Fixtures sem atividade contêm ditado e exigem nenhuma chamada de provedor, na hora e no dia; não houve captura ambiente. Histórico (oito testes) e timeline (13 testes) compõem os 2.069 aprovados/7 ignorados de frontend. O CI automático 41582917424 da fonte `9bc475d0` aprovou formato, Clippy e a suíte Rust completa no macOS (2.474 aprovados únicos/16 ignorados); não somar a contagem das rodadas locais novamente. Recibos em `.tmp/verify/T7/mac-native-20261102/`, `.tmp/verify/T7/parent-checks-20261102/3/` e `.tmp/verify/T7/mac-signature-9bc475d0-20261103/ci-rust-job.log`.

4.2 permanece aberto: não foi observado ditado real em dois apps nem geração do relatório pela UI. Falta concessão humana de Acessibilidade e entrada sintética apropriada; não se forjou grant nem se autorizou captura ambiente como fixture. A fonte2dbe passou no CI41589762530; a revisão do checkpoint foi manual11, não aprovação automática. A sessão gráfica bloqueou durante a continuação737; os probes de readiness registrados no app não qualificam a prova de ditado nem autorizam apagar seus possíveis dados temporários. Arquivo integral não é elegível enquanto o aceite obrigatório faltar.
