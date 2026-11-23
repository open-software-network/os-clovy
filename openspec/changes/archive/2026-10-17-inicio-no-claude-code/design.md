# Design

## Context

Motivação em `proposal.md`. Estado atual (mapeado em 2026-10-16 sobre `57afef38`):

- Frontend: o Início é `AgentWorkspace` com `homeMode` (`src/app/app-workspace-view.tsx:235-272`). `submitHomeMessage` (`AgentWorkspace.tsx:1965-2214`) chama `clovyHomeChat` (`src/lib/tauri.ts:74`) com `historyContext` montado de `localStorage` (`src/lib/clovy-home.ts:165-211`). Turnos diretos ficam em `localStorage["clovy:home:direct-turns:v1"]` e cartões em `"clovy:home:task-handoffs:v1"` (`src/components/agent/home-thread.ts`). `ensureHomeSession` (`AgentWorkspace.tsx:1790-1810`) cria uma sessão `title: "Home"`, `model: "open-software/auto"` só para ter um id estável, guardado em `"clovy:home:session-ids:v1"`; a barra lateral e o `App.tsx` filtram esse id (`Sidebar.tsx:164-165,417`, `App.tsx:372-375,565-568`).
- Rust: `clovy_home_chat` / `june_home_chat` (`src-tauri/src/clovy_api.rs:2495-2686`, registrados em `lib.rs:240-241`) montam prompt de sistema com persona (`clovy_api.rs:1933-1948`), resposta fixa de identidade (`:2248-2305`), heurística de informação atual (`:2307-2393`), ferramenta `start_task` e streaming SSE próprio (`:2004-2131`), tudo via `proxy_agent_chat_completions` para `__june_auto_generation__:0`.
- Sessões CLI: `createSession` + `startRun` com `model: "__clovy_cli_engine__:claude"` passam por `start_agent_run` → `start_cli_run` (`agent_runtime/api.rs:720-910`) → `start_cli_turn` (`chat_engine/mod.rs:263-318`), que grava o pedido em `agent_items`, retoma com `--resume` ou prefixa histórico (`chat_engine/turn.rs:184-199,639-720`). O motor padrão vem de `chat_default_engine` (`chat_engine/mod.rs:382-406`, `src/lib/chat-engine.ts:88-115`).

## Goals / Non-Goals

**Goals:**
- Um único caminho de turno CLI para o Início, o mesmo das sessões (ADR-0062: nunca um segundo caminho de CLI).
- Remover todo o código do Início que só existia para a triagem na Clovy API.

**Non-Goals:**
- Ver `proposal.md` (Non-goals). Também fora: mudar o visual do Início (saudação, sugestões vazias, layout), que continua igual.

## Decisions

### D1. O Início é uma sessão CLI comum, escondida da lista

O envio no Início faz o que a Nova sessão faz depois de ter uma sessão: `startRun({ sessionId: homeSessionId, prompt, model: <motor padrão>, safetyMode: "sandboxed", attachments })`. A conversa é renderizada a partir dos itens da sessão, com a mesma projeção de itens e cartões das sessões CLI (streaming, aprovação, clarificação, erro).

Alternativa rejeitada: manter a triagem e só trocar o modelo por um CLI one-shot (`claude -p` sem ferramentas, devolvendo JSON de `reply`/`task`). Mantém dois caminhos de CLI, um contrato de JSON frágil e a duplicação de histórico em `localStorage`; o dono pediu "tudo no Claude Code", sem triagem.

### D2. Sessão do Início adota o motor padrão

`ensureHomeSession` cria a sessão com o motor padrão (`chatDefaultEngine().modelId`) em vez de `open-software/auto`. Para uma sessão do Início já existente com modelo antigo, o próprio `startRun` com o modelo do motor padrão decide o motor do turno (o mesmo comportamento de uma sessão nativa que recebe a próxima mensagem no CLI, `turn.rs:616-657`); se `start_agent_run` usar o modelo da sessão e não o do pedido, a sessão do Início é atualizada para o motor padrão antes do primeiro turno. O construtor confirma qual dos dois vale e testa o caso da sessão antiga.

O id continua no `localStorage` (`clovy:home:session-ids:v1`) e os filtros da barra lateral e do `App.tsx` não mudam.

### D3. Importação única do histórico antigo

Na abertura do Início, se `clovy:home:direct-turns:v1` tem turnos e a sessão do Início ainda não recebeu a importação, os turnos (usuário e assistente, em ordem, com a hora original) são gravados como itens da sessão por um comando Rust de importação, idempotente por marcador gravado no banco da sessão (não no `localStorage`, para não importar duas vezes se o webview perder o armazenamento e o recuperar). Os turnos importados entram no histórico prefixado do próximo turno, já que não há conversa de CLI para retomar. O `localStorage` antigo fica intacto como recuperação (ADR-0037).

Os cartões de `clovy:home:task-handoffs:v1` continuam lidos e renderizados como hoje, intercalados por hora com os itens da sessão; nenhum cartão novo é criado.

Alternativa rejeitada: descartar o histórico antigo. Perde dados do usuário sem necessidade.

### D4. Sem persona, sem identidade fixa

O turno do Início é igual ao de qualquer sessão CLI: sem `--append-system-prompt`, sem prompt de sistema do Clovy, sem resposta fixa de identidade. Escolha do orquestrador, alinhada com "um só caminho de turno"; o dono pode pedir a persona depois, numa change própria que a aplique a todas as sessões CLI.

### D5. Remoção

Sai de `clovy_api.rs`: `clovy_home_chat`, `june_home_chat`, `ClovyHomeChat{Message,Request,Task,Event,Response,Stream}`, `clovy_home_chat_system_prompt`, `CLOVY_HOME_*`, `CLOVY_IDENTITY_REPLY`, `clovy_identity_reply`, `clovy_home_requires_current_information`, `require_clovy_home_current_research`, `clovy_home_chat_task`, `buffered_clovy_home_chat_response` e testes deles. `proxy_agent_chat_completions` fica (segurança de imagem, `clovy_api.rs:2691`). `persona.rs` fica se ainda tiver outro usuário; se o Início era o único, o construtor registra isso e não o apaga nesta change (pertence à remoção do nativo, tarefa 4.2 de `cli-substitui-agente-nativo`).

Frontend: `clovyHomeChat` em `tauri.ts`, o envio por triagem em `AgentWorkspace.tsx`, `startHomeTask`, `buildClovyHomeConversationContext` e o que só a triagem usava em `clovy-home.ts`/`home-thread.ts`, os códigos `home_chat_*` em `errors.ts` e as mensagens em `i18n/messages/lib.ts` que ficarem sem uso. Testes que fixavam a triagem são apagados, não reescritos para o novo caminho.

### D6. Documentação

ADR-0064 "Home runs on the default CLI engine" substitui a frase do Início na decisão 3 e a consequência "Home triage still needs Clovy API" da ADR-0062, que ganha nota `superseded in part by ADR-0064`. `docs/llm-providers.md` e `CONTEXT.md` deixam de descrever triagem do Início.

## Risks / Trade-offs

- [Latência: a partida do CLI por turno leva segundos, mais que o streaming da Clovy API] → Aceito pelo dono ao pedir tudo no Claude Code; o indicador de turno em andamento das sessões CLI aparece já no envio.
- [O Início passa a fazer trabalho de verdade (arquivos, shell) em `sandboxed`] → É o mesmo modo e as mesmas aprovações da Nova sessão; sem pré-concessão de ferramentas.
- [Sessão do Início antiga com modelo `open-software/auto`] → D2 cobre e exige teste do caso.
- [Turnos importados com formato inesperado no `localStorage`] → Importação ignora entradas ilegíveis, conta e registra quantas, e nunca falha a abertura do Início.

## Migration Plan

Sem migração de esquema SQL prevista: itens importados usam as tabelas de sessão existentes; o marcador de importação fica num item ou campo já disponível da sessão (o construtor escolhe; se precisar de coluna, migração nova sem apagar arquivos antigos). Rollback: reverter o merge; o `localStorage` antigo continua intacto, então a versão anterior volta a mostrar o histórico.

## Implementation notes (2026-10-17)

- **D2 confirmado:** `start_agent_run` usa o modelo do pedido e grava esse modelo na sessão (`UPDATE agent_sessions SET model = ?` antes de `start_cli_run`). A sessão antiga `open-software/auto` adota o motor no próprio envio, sem migração; sessões novas do Início já nascem no motor padrão.
- **Um só caminho de envio:** o Início usa o mesmo `submit` das sessões; só a obtenção da sessão (`ensureHomeSession`) e o modelo do turno (`homeEngineModelId`, lido a cada envio) mudam. Isso traz de graça a fila e o direcionamento de mensagens enviadas durante um turno e a linha de mensagem não enviada das sessões. Sem CLI instalado, o Início pede `__clovy_cli_engine__:claude` para que o turno falhe com o erro de motor ausente das sessões CLI.
- **Marcador da importação:** prefixo `home-import:<id da sessão>:` no `external_id` dos itens importados (nenhuma coluna nova). Itens sem texto (turno só com cartão, só anexo) não são importados.
- **Cartões antigos:** lidos só para exibição; um cartão que ficou em `starting` aparece como falho em memória, sem regravar o `localStorage`. Sem novos cartões, o botão de tentar de novo dos cartões falhos sai junto com `startHomeTask`.
- **Removido por ser só da triagem:** o demo de desenvolvimento do Início (`__homeDemo`, `homeDemoReply`), o limite próprio de 64.000 caracteres do Início e a linha de mensagem não enviada específica do Início (substituída pela das sessões).
- **`persona.rs` fica:** ainda é usado pelas instruções do agente nativo (`agent_runtime/api.rs`) e pelas rotinas (`routines.rs`).
- **Documentação:** além de `CONTEXT.md` e `llm-providers.md`, `docs/home-assistant.md` (que descrevia a triagem inteira) foi reescrito e `docs/index.md` ganhou a ADR-0064.
