# Tasks

> Executor: Codex na agents-us, seguindo `/implement`, TDD e, no frontend, `~/.agents/skills/impeccable/SKILL.md`. Base: branch `feat/clovy-composer-permissoes-20261106`, commit `c42b7f34`. O orquestrador compila o app de QA no Mac e conduz a prova com o dono.

## Global Constraints

- Sem dependência nova; sem mudar ditado, broker (exceto a origem do modo) ou paridade dos runtimes.
- `bypassPermissions`/`--yolo` continuam proibidos; acesso total só no Codex em "Sem restrições".
- Não substituir o app real, não ler dados pessoais, não autenticar provedores.

## 1. Modo único

- [x] 1.1 RED: Rust `create_run` deriva `approvalMode` de `safety_mode` (unrestricted → bypass, sandboxed → approvals), inclusive retry; rotinas continuam em aprovação no broker.
- [x] 1.2 GREEN no backend; remover o comando `set_agent_session_approval_mode` e ajustar `protocol.rs`; atualizar testes Rust afetados (`repository.rs:1289`, `api.rs:2482`, `migrations.rs:2235`, `tests/agent_sessions_persistence.rs`).
- [x] 1.3 Remover o select "Aprovação de ferramentas" e todo o estado, textos, CSS, ponte Tauri e tipo; reescrever/remover testes TS afetados (`agent-approval-first-run.test.tsx`, `conversation-external-events.test.tsx:171`, `home-conversation.test.tsx:131`, `chat-engine-presentation.test.tsx:160`).

## 2. Codex com acesso total

- [x] 2.1 RED/GREEN em `policy_tests.rs`: unrestricted + Codex → `danger-full-access`; sandboxed nunca; Claude/Antigravity sem flags proibidas. Ajustar `FORBIDDEN` só para esse caso. Atualizar ADR 0063, `docs/mcp-server.md` e o texto do menu se preciso.

## 3. Composer compacto

- [x] 3.1 Remover a altura mínima do hero e alinhar ao composer de Rotinas, preservando "+", sandbox, motor, microfone, menus `/`/`@` e crescimento até 200 px. Testes de CSS/layout existentes atualizados.

## 4. Antigravity em Atividade

- [x] 4.1 RED/GREEN de UI: escolher em Atividade uma CLI sem nível medido roda o teste e aplica se passar; login ausente aponta "Autenticar no Clovy"; cota/prazo com mensagem própria; nível `none` mantém a mensagem atual.

## 5. Prova e fechamento

- [x] 5.1 Suítes afetadas (vitest focado, `cargo test` dos módulos tocados, Clippy `-D warnings`, `tsc`); app de QA no Mac; o dono confere os três pontos.
- [x] 5.2 Revisão pelo gate `/Users/guilhermevarela/orchestrator.v5/scripts/review-pair.sh --gate` ou revisão manual de contingência registrada, com scanner de segredos; commit de escopo.
- [x] 5.3 Atualizar `docs/llm-providers.md`, `docs/mcp-server.md`, ADR 0063 e `AGENTS.md`.

## Evidências (2026-11-06)

- Worker Codex (agents-us), relatório `.tmp/verify/composer-modo-unico-e-agy-atividade/implementation/report.md`, patch SHA256 `5f76d4657a4dafbc17f725f62e326faa290f4a688fafc8bc2dc2dc9e9c5e5908`. Frontend completo na box: 215 arquivos/2124 testes passaram; `pnpm check` e `typecheck` ok; RED composer 4 falhas, Atividade 5 falhas; prova de navegador com capturas.
- Mac (árvore de integração `qa-integracao-20261106`, com o patch do ditado): `cargo test --lib -- agent_sessions chat_engine approval_broker mcp_server llm::` 506 passaram/0 falhas; `--test agent_sessions_persistence` 16 passaram; `cargo clippy --locked --lib --tests -D warnings` exit 0. Logs em `qa-integracao-20261106/.tmp/verify/qa-integracao-20261106/`.
- App de QA integrado `dev.clovy.dr13469713` testado pelo dono: "parece que ta tudo ok". Limite: relato geral; autenticação real do Antigravity não verificada pelo orquestrador.
- Gate `review-pair.sh --gate`: rodada 1 exit 6 com 2 achados CodeRabbit. (a) 5.2 aberta: esperado, é o próprio gate. (b) hero ainda supunha 640px no aviso de imagem, menu de modo e menu de motor, com `--content-max` de 760–1000px: procedente, corrigido em `app.css` derivando de `--content-max` (vitest de layout 97/97). Rodada 2: exit 0, OpenQodex e CodeRabbit concluídos, 0 achados, gitleaks limpo. Recibos `.tmp/verify/composer-modo-unico-e-agy-atividade/review-pair/{1,2}/summary.md`.
