# Tasks

Gates de `AGENTS.md` ("Verification") em cada grupo para o que ele tocou. Prova nativa só no grupo 4, com identificador próprio conforme "Functional proof".

## 1. Início no motor CLI

- [x] 1.1 `ensureHomeSession` cria a sessão do Início com o motor padrão (`chatDefaultEngine().modelId`) e resolve o caso da sessão antiga `open-software/auto` (design D2: confirmar se `start_agent_run` usa o modelo do pedido ou da sessão e atualizar a sessão se preciso). Verificar com teste de componente que falha antes: sessão antiga + envio → `startRun` com `__clovy_cli_engine__:claude`. Feito: `home-conversation.test.tsx` (sessão antiga → `startRun` com o motor padrão; criação da sessão no motor; Codex), D2 confirmado em `start_agent_run` (ver design.md, Implementation notes).
- [x] 1.2 Envio no Início chama `startRun` na sessão do Início (`safetyMode: "sandboxed"`, anexos) e renderiza a conversa a partir dos itens da sessão com a projeção das sessões CLI (streaming, aprovação, erro). Verificar: testes de componente para resposta em streaming, cartão de aprovação no Início, nenhuma sessão nova criada, e sessão do Início fora da barra lateral. Feito: testes de streaming, cartão de aprovação, nenhuma sessão nova e barra lateral sem a sessão do Início em `home-conversation.test.tsx`.
- [x] 1.3 Erros do turno no Início usam o mapeamento das sessões CLI e preservam a mensagem no compositor (nenhum CLI instalado, turno falho). Verificar com testes de componente. Feito: testes de nenhum CLI instalado (erro + texto no compositor) e de turno falho com "Try again" → `retry_agent_run`.
- [x] 1.4 Teste de integração Rust: dois turnos na sessão do Início com CLI falso; o segundo recebe `--resume` ou o histórico prefixado; servidor falso da Clovy API recebe zero chamadas. Verificar com `cargo test` do teste novo. Feito: `chat_engine_home_session_runs_two_turns_on_the_cli_without_clovy_api` (histórico prefixado no 1º, `--resume` no 2º, zero chamadas à Clovy API falsa).

## 2. Histórico antigo

- [x] 2.1 Comando Rust de importação única dos turnos de `clovy:home:direct-turns:v1` como itens da sessão do Início, em ordem e com hora original, idempotente por marcador no banco, ignorando entradas ilegíveis (design D3). Verificar com teste de integração: importa, reimporta sem duplicar, entrada ilegível não falha. Feito: `import_legacy_home_turns` (`agent_runtime/home_import.rs`), 4 testes `home_import_*`.
- [x] 2.2 Frontend chama a importação na abertura do Início e continua renderizando os cartões de `clovy:home:task-handoffs:v1` intercalados por hora, abrindo a sessão de destino; `localStorage` não é apagado. Verificar com teste de componente usando fixture de `localStorage`. Feito: importação na abertura e cartões antigos em `home-conversation.test.tsx` e `clovy-home.test.ts`; `localStorage` intacto.

## 3. Remoção e documentação

- [x] 3.1 Remover `clovy_home_chat`, `june_home_chat` e todo o código só da triagem em `clovy_api.rs` e `lib.rs` (design D5), mantendo `proxy_agent_chat_completions`; registrar se `persona.rs` ficou sem usuário. Verificar: `cargo clippy --all-targets -D warnings` e `pnpm test:rust` sem falhas; busca sem referências. Feito: bloco do Início removido de `clovy_api.rs` e `lib.rs`; clippy `-D warnings` e `pnpm test:rust` sem falhas; `persona.rs` ainda tem usuários (agente nativo, rotinas).
- [x] 3.2 Remover do frontend `clovyHomeChat`, `startHomeTask`, `buildClovyHomeConversationContext` e o que só a triagem usava, códigos `home_chat_*` e mensagens i18n sem uso; apagar os testes que fixavam a triagem. Verificar: `src/test/tauri-command-registry.test.ts` verde, `pnpm check`, `pnpm typecheck` e vitest completo sem falhas novas. Feito: `clovyHomeChat`, `startHomeTask`, `buildClovyHomeConversationContext`, códigos `home_chat_*`, mensagens sem uso e testes da triagem removidos; guarda de comandos, `pnpm check`, `pnpm typecheck` e vitest (200 arquivos, 0 falhas) verdes.
- [x] 3.3 ADR-0064 "Home runs on the default CLI engine", nota `superseded in part by ADR-0064` na ADR-0062, `docs/llm-providers.md` e `CONTEXT.md` sem triagem do Início. Verificar: busca por `home_chat`, `june_home_chat` e "triage" em `docs/`, `CONTEXT.md` e `AGENTS.md` só em ADRs históricas. Feito: ADR-0064, adendo na ADR-0062, `llm-providers.md`, `CONTEXT.md`, `home-assistant.md`, `docs/index.md`; busca sem `home_chat`/triagem do Início fora das ADRs 0062/0064.

## 4. Prova

- [x] 4.1 `make verify` sem falhas novas. Feito: exit 0, vitest 200 arquivos e 2012 testes, 48 suítes Rust ok, log em `.tmp/verify/inicio-no-claude-code/run-20261017/make-verify.log`.
- [x] 4.2 Prova nativa com identificador próprio e Clovy API local sem chave da OpenRouter: enviar "eae" e uma segunda mensagem que depende da primeira no Início, ver as respostas do Claude Code, reiniciar o app e ver a conversa; conferir no banco da cópia de perfil os itens da sessão do Início e no log da Clovy API nenhuma chamada a `/v1/chat/completions`. Evidência em `.tmp/verify/inicio-no-claude-code/run-<id>/report.md`. Feito pelo dono no Clovy Dev (perfil `-dev`, não cópia), Clovy API sem chave da OpenRouter, aceite "tudo funcionando" em 2026-10-17; banco: sessão do Início em `__clovy_cli_engine__:claude`, 3 mensagens antigas importadas e resposta do Claude Code que cita o histórico; `run-20261017-owner/report.md`. Não provado no app: segunda mensagem dependente da primeira (coberto por teste Rust) e contagem de requisições na Clovy API (ela não registra requisições).

## Workflow follow-up

- Revisão por harness diferente do autor, aceite do dono, `/openspec-archive-change`, `/sync` e `/cleanup`.
- A linha do Início em `openspec/changes/cli-substitui-agente-nativo/design.md` ("`POST /v1/chat/completions` continua (Início, ...)") fica desatualizada; ajustar por `/openspec-update-change` naquela change.
