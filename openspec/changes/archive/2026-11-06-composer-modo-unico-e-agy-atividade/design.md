# Design

Referências de linha do commit `c42b7f34` (investigação somente leitura).

## 1. Um único controle de modo

Hoje:
- Select de aprovação: `AgentWorkspace.tsx:2716-2739`, props `approvalMode*`/`onApprovalModeChange` (`:2017-2057`, `:2505-2551`), estado `draftApprovalMode`/`draftApprovalTouched`/`pendingApproval` (`:490-499`, resets `:552-555`, `:812-815`), salvar antes do `startRun` (`:1187-1188`, `:1324-1352`), `publishSavedApproval` (`:667-679`). Textos `i18n/messages/agent.ts:5,210-218`; CSS `.agent-approval-control` (`app.css:5434-5450`); `tauri.ts:64-65` → `set_agent_session_approval_mode` (`api.rs:298-308`, `lib.rs:208`).
- Seletor de segurança: só no hero (`AgentWorkspace.tsx:2802-2823`, menu `:2935-3000`); `start_agent_run` grava `agent_sessions.safety_mode` antes de `create_run` (`api.rs:662-688`).
- `create_run` (`repository.rs:240-265`) copia `s.approval_mode` para `run_config_json.approvalMode`; o broker lê essa cópia (`approval_broker/mod.rs:225-261`).

Decisão:
- `create_run` deriva a cópia por execução: `CASE WHEN s.safety_mode = 'unrestricted' THEN 'bypass' ELSE 'approvals' END`. A ordem existente (gravar `safety_mode` → `create_run`) garante o valor da execução atual; retry passa pelo mesmo caminho.
- Remover o select, o estado de rascunho, o comando Tauri e o tipo no contrato. A coluna `approval_mode` fica sem uso, sem migração destrutiva (documentar como legado); `protocol.rs:175-206` passa a refletir o valor derivado.
- Broker inalterado: rotinas/execuções sem pessoa continuam em aprovação (`mod.rs:247-255`, `:270`); esclarecimentos continuam esperando o usuário; recibos de bypass continuam exigindo execução ativa (`store.rs:53`).
- Home/Mensagens é sempre `sandboxed` (`:778`, `:1404-1408`) e continua com aprovações. Seletor continua só na conversa nova.
- Conversas antigas com `approval_mode='bypass'` e `sandboxed` passam a pedir aprovação (mais seguro, aceito).

## 2. Codex com acesso total em "Sem restrições"

`chat_engine/policy.rs`: Claude `:167-183`, Codex `:187-213`, Antigravity `:126`, lista `FORBIDDEN` `:75-88` (testes `policy_tests.rs`, incluindo `:311`).

Decisão do dono: Codex em `unrestricted` usa `sandbox_mode = "danger-full-access"` (com `approval_policy = "never"`). `FORBIDDEN` passa a permitir `danger-full-access` somente nesse caminho; `bypassPermissions`, `--yolo` e equivalentes continuam proibidos. Claude continua `acceptEdits` + `--permission-prompt-tool` via broker (bypass derivado já libera). Antigravity sem mudança de flags. Testes de política: sandboxed nunca recebe acesso total; unrestricted recebe só no Codex. Rotinas com "Sem restrições" + Codex também recebem acesso total (escolha explícita por rotina); documentar.

## 3. Composer compacto

Causa: `.agent-composer[data-hero="true"] .agent-composer-editor { min-height: 76px; }` (`app.css:8082-8084`). Rotinas: `RoutinesView.tsx:1070-1157`, `app.css:19626-19650`. Remover a regra; alinhar medidas visuais ao de Rotinas (ícone de enviar, largura) sem perder "+", sandbox, motor e microfone. Conferir menus `/` e `@` (ancoram no retângulo de `.agent-composer-box`) e o pulo do título do hero ao crescer. Seguir `/impeccable`. Testes: `home-polish-css.test.ts`, `agent-workspace-runtime.test.tsx:2437,2494`, `composer-editor-slash-menu.test.tsx`.

## 4. Antigravity em Atividade

Hoje: `set_usage` → `require_json_schema` lê o nível medido (`registry.rs:235,505-527`); só `llm_test_provider` grava (`commands.rs:81-101`, `test_cli` `mod.rs:470-491`); erros de cota/login/prazo não gravam nada; UI mostra `StructuredOutputInsufficient` (`LlmProvidersSection.tsx:153-169`, `i18n/messages/lib.ts:534-535`).

Decisão: ao escolher em Atividade uma CLI instalada, one-shot e sem nível medido, o frontend mostra estado "testando", chama o teste do provedor e, se o nível gravado suportar JSON schema, reaplica `set_usage`. Falha de autenticação → mensagem que aponta "Autenticar no Clovy"; cota/prazo → mensagem própria; nível `none` → mensagem atual. Nada muda no nível fixo do Agy nem em `--json-schema`. Teste de UI com provedor falso; teste Rust só se o erro precisar de variante nova.
