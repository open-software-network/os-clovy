# Proposal

## Why

O dono decidiu em 2026-10-15 que tudo o que roda no agente nativo do Clovy (sidecar OpenAI Agents SDK em `agent-runtime/` + `src-tauri/src/agent_runtime/`) passa a rodar nos motores CLI (Claude Code e Codex, via `src-tauri/src/chat_engine/`) e que, depois, o agente nativo é apagado. A fase 1 (CLI como padrão em sessões novas, Início, chat da nota, companion e rotinas com política só-leitura) está em curso em outra worktree (`cli-20261015/cli-routing`) e não é redesenhada aqui.

Falta o que impede o corte: os CLIs só recebem o servidor `clovy-mcp`, que é **só leitura** (`src-tauri/src/mcp_server/mod.rs:1-11`, `src-tauri/src/mcp_server/tools.rs:122-129`), não sabe a que sessão uma chamada pertence (`src-tauri/src/mcp_server/protocol.rs:1-4`) e não tem como pedir aprovação ao usuário: um turno `-p` que precisa de permissão falha com "a chat turn cannot ask for it" (`src-tauri/src/chat_engine/turn.rs:459-466`). Mais de 50 ferramentas existem só no catálogo nativo (`src-tauri/src/agent_runtime/api.rs:2360-2461`, `src-tauri/src/agent_runtime/native_connectors.rs:38-151`), e o modo de segurança da sessão não chega aos CLIs (`src-tauri/src/chat_engine/mod.rs:168-175` não tem campo de segurança).

## What Changes

- **Vínculo de turno no MCP**: cada turno CLI recebe um token de turno de uso único nos argumentos do `clovy-mcp`; o app sabe a sessão, a execução e a política de cada chamada. Clientes externos sem token continuam vendo só as ferramentas de leitura (ADR-0058 inalterada para eles).
- **Broker de aprovação via MCP** (peça central): ferramentas de escrita pedem aprovação ao app pelo socket existente; o app mostra o cartão de aprovação na sessão (desktop, e o companion para o escopo da ADR-0053); a chamada da ferramenta espera com timeout; negação, expiração ou cancelamento viram erro de ferramenta. Clarificação usa o mesmo mecanismo. Para o Claude Code, os prompts de permissão das ferramentas próprias do CLI também passam pelo broker (`--permission-prompt-tool`, sujeito ao spike).
- **Paridade de ferramentas no `clovy-mcp`**: notas/ditados por id com paginação, memória, mídia, gravador, browser use, computer use, vault Obsidian, skills, CRUD de rotinas, conectores (Gmail, Calendar, GitHub, Notion), interações. Nomes fixados no delta `clovy-mcp-server`.
- **Gateway de MCP**: Linear e os servidores MCP do usuário entram nos turnos CLI através do `clovy-mcp`, mantendo credenciais no Keychain e a política do host (ADR-0039). O repasse direto no `--mcp-config` fica como alternativa para decisão do dono.
- **Política de permissões dos CLIs**: o modo da sessão (`sandboxed`/`unrestricted`, `src-tauri/src/agent_runtime/domain.rs:4-27`) vira flags de Claude e Codex para turnos assistidos e não assistidos, sem nunca usar flags de bypass.
- **Remoção do nativo** (fase final): sidecar, host do runtime, ferramentas nativas, interrupções do SDK, picker de modelo nativo, motor "endpoint", chamadas desktop às rotas só do nativo, docs e testes; com critério de corte explícito.

## Capabilities

### New Capabilities

`openspec/specs/` está vazio; tudo entra como ADDED.

- `clovy-mcp-server`: vínculo de turno, ferramentas de escrita só em turnos vinculados, catálogo de paridade com nomes fixos, gateway de MCP.
- `cli-approval-broker`: pedido, exibição, resposta, timeout e auditoria de aprovações e clarificações vindas de turnos CLI.
- `cli-engine-permissions`: mapeamento do modo de segurança para flags de Claude e Codex, anexos/visão e limites de tempo das ferramentas.
- `native-agent-removal`: critério de corte, o que é apagado, o que permanece e como o histórico sobrevive.

### Modified Capabilities

Nenhuma spec principal existente. Interage com `clovy-mcp-server` das changes `add-activity-intelligence` e `default-capture-and-session-mcp` (ferramentas de leitura e ligado por padrão continuam valendo).

## Impact

- Rust: `src-tauri/src/mcp_server/**` (handshake, registro de turnos, grupos de ferramentas), novo `src-tauri/src/approval_broker/`, `src-tauri/src/chat_engine/{cli,turn,mod}.rs`, consumidores de `agent_mcp.rs`, `browser_broker.rs`, `computer_use.rs`, `agent_recorder.rs`, `routines.rs`, `connectors/*`.
- Frontend: cartões de aprovação para sessões CLI (`src/components/agent/chat-turns/AgentActionCards.tsx`), companion; na fase final, remoção do picker nativo e dos caminhos de motor "clovy"/"endpoint".
- clovy-api: **nenhuma rota é removida** (ADR-0021); o desktop só deixa de chamar `/v1/web/*` e o caminho de agente de `/v1/chat/completions`.
- ADRs: rascunho `docs/adr/0063-cli-turns-get-write-tools-through-an-approval-broker.md` (proposto; supera a parte "tudo é só leitura" da ADR-0058 para turnos vinculados e, para CLIs, a frase "June ships no MCP server of its own" da ADR-0040). A remoção final exige um ADR próprio na fase 4 que supera ADR-0038, ADR-0040, ADR-0013 e as partes de runtime da ADR-0039 e da ADR-0053.
- Fora de escopo: fase 1 (outra worktree); pi, agy e cursor-agent continuam sem ferramentas do Clovy; nenhuma rota do clovy-api é apagada; nenhuma credencial vai para a configuração de um CLI.
