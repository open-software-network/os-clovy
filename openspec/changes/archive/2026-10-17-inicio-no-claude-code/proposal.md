# Proposal

## Why

A conversa do Início é o único chat do desktop que ainda depende da Clovy API: cada mensagem vai para `POST /v1/chat/completions` (OpenRouter) para uma triagem leve, e só a tarefa entregue roda no CLI (ADR-0062, decisão 3). Sem chave da OpenRouter na Clovy API, o Início falha com `home_chat_failed` (`openrouter_not_configured`), enquanto a Nova sessão funciona no Claude Code. O dono decidiu em 2026-10-16 que o Início roda inteiro no Claude Code, sem Clovy API.

## What Changes

- Toda mensagem do Início vira um turno do motor CLI padrão (Claude Code; Codex só quando o Claude Code não está instalado), pelo mesmo caminho das sessões (`start_agent_run` → `start_cli_run` → `start_cli_turn`), na sessão persistente do Início que já existe e continua fora da lista lateral.
- A conversa do Início passa a ser gravada no banco como itens da sessão do Início (como qualquer sessão CLI), e não mais só no `localStorage`; a continuidade vem do `--resume` / histórico prefixado do motor.
- **BREAKING** Sai a triagem: o Início não chama mais a Clovy API, não usa mais o modelo `__june_auto_generation__`, a resposta fixa de identidade, a heurística de "informação atual" nem a ferramenta sintética `start_task`. O Claude Code responde e trabalha ali mesmo, com as ferramentas e o modo de segurança (`sandboxed` por padrão) de qualquer sessão CLI.
- **BREAKING** Comandos `clovy_home_chat` e `june_home_chat` removidos, com DTOs, streaming SSE, prompt de sistema do Início e códigos de erro `home_chat_*` que só eles usavam.
- Histórico antigo preservado: as mensagens diretas do Início gravadas no `localStorage` entram uma única vez como itens da sessão do Início; os cartões de tarefas já entregues continuam visíveis e abrindo a sessão de destino.
- Sem CLI instalado ou com o CLI sem login, o Início mostra o mesmo erro acionável das sessões CLI, com a mensagem preservada no compositor.
- ADR nova (0064) substitui a parte do Início na decisão 3 da ADR-0062; a linha do design de `cli-substitui-agente-nativo` que mantinha o Início na Clovy API deixa de valer.

## Capabilities

### New Capabilities

- `home-conversation`: o que o Início faz com uma mensagem do usuário: motor que responde, onde a conversa é guardada, continuidade, histórico migrado e erros.

### Modified Capabilities

(nenhuma: não há specs canônicas em `openspec/specs/`.)

## Non-goals

- Outras dependências da Clovy API ficam como estão e são fora deste corte: transcrição (Groq), geração e limpeza de notas quando o usuário escolheu Clovy API, imagem e vídeo (fal), triagem de segurança de prompt de imagem, sugestão de título de sessão (já cai para título local), compartilhamento, companion, relatórios de problema e P3A. Tirar a Clovy API dessas superfícies é outra change.
- Nenhuma rota da Clovy API é removida (ADR-0021); `POST /v1/chat/completions` só perde o chamador do Início.
- Persona do Clovy (`persona.rs`) não é injetada no Claude Code: o Início responde como o motor CLI responde em qualquer sessão.
- Sem seletor de motor nem de modo de segurança no compositor do Início.

## Impact

- Rust: `src-tauri/src/clovy_api.rs` (remoção do bloco do Início, ~linhas 1930-2690, mantendo `proxy_agent_chat_completions` para a segurança de imagem), `src-tauri/src/lib.rs` (`generate_handler!`).
- Frontend: `src/components/agent/AgentWorkspace.tsx` (`submitHomeMessage`, `ensureHomeSession`, `startHomeTask`), `src/components/agent/home-thread.ts`, `src/lib/clovy-home.ts`, `src/lib/tauri.ts`, `src/lib/errors.ts`, `src/i18n/messages/lib.ts`, testes do Início (`agent-workspace-runtime.test.tsx`, `clovy-home.test.ts`).
- Docs: ADR-0064 nova, adendo na ADR-0062, `docs/llm-providers.md`, `CONTEXT.md` (termo Início/triagem).
- Usuário: o Início fica mais lento para responder (partida do CLI por turno) e passa a depender do Claude Code instalado e logado.
