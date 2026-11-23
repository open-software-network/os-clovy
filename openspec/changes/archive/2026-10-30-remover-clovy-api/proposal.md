# Proposal

## Why

O dono decidiu em 2026-10-28 que o app nunca mais chama o servidor Clovy API (`CLOVY_API_URL`, padrão `127.0.0.1:8080`, rotas `/v1/*`) e que essa dependência sai de tudo. O chat é só do Claude Code. Fica o que roda na máquina, as chamadas diretas aos provedores do próprio usuário com as chaves dele, e as ferramentas do agente Clovy (servidor MCP `clovy`).

Hoje o release não traz esse servidor, então tudo que ainda depende dele já falha em silêncio. O seletor "Modelo de texto", por exemplo, mostra só o modelo salvo porque a lista vem de `GET /v1/models` (`providers/mod.rs` ~1408-1437). Esta change também absorve o pedido de listar o catálogo do OpenRouter nesse seletor.

## What Changes

- **BREAKING** Nenhuma chamada ao Clovy API em nenhum caminho do app. Saem `clovy_api_url`, o token local (`local_identity.rs`), o crate `clovy-api/`, o workflow e os alvos de `make`/`pnpm` dele, e os testes que fixam as rotas `/v1/*`.
- Transcrição de nota, prévia ao vivo e ditado vão sempre direto à Groq. Sem a chave Groq, a gravação e o ditado mostram que a chave falta e onde salvá-la. A prévia ao vivo só liga com a chave salva.
- A geração e a limpeza da nota com motor OpenRouter vão sempre direto à OpenRouter. A opção de motor "Clovy" passa a se chamar "OpenRouter". Sem a chave, a nota mostra que a chave falta. Auto (`open-software/auto`) e o controle de custo e qualidade saem.
- A limpeza do ditado vai só à OpenRouter. Sem a chave, o texto sai sem limpeza e com aviso.
- O catálogo do "Modelo de texto" vem de `GET https://openrouter.ai/api/v1/models`. Todos os modelos que respondem texto ficam selecionáveis, sem exigir ferramentas, porque o chat não usa esse modelo. A lista recarrega quando a chave muda.
- Escolher um modelo de texto não mexe mais no motor do chat. A descrição do seletor diz que ele serve às notas e indica qual motor gera as notas hoje.
- O chat aceita só motores CLI. O motor "Clovy" (agente nativo) e o endpoint como modelo do agente saem. Isso conclui a fase 4 da change `cli-substitui-agente-nativo`, que esta change pressupõe.
- As ferramentas de mídia do agente Clovy (`generate_image`, `edit_image`, `generate_video`) passam a chamar a fal direto, com a chave fal do usuário. Sem a chave, a ferramenta devolve erro dizendo qual chave falta.
- Saem as funções que só o servidor fazia:
  - compartilhamento privado de notas e sessões (links `/s/{id}`);
  - botão "Verificar" em Sobre;
  - classificador de prompt de imagem por modelo (fica a triagem por palavras);
  - títulos de sessão por modelo (código morto);
  - busca e leitura web do agente nativo;
  - mensagens de crédito e saldo;
  - flag `provider_configured`.
- Rotinas não assistidas rodam só em CLI. Sem Claude nem Codex instalado, falham com erro claro.
- Os prompts `note_generate.md` e `note_transcript_cleanup.md` mudam de `clovy-api/` para `src-tauri/src/prompts/`.

## Capabilities

### New Capabilities

- `text-model-catalog`: origem, recarga e uso da lista de modelos de texto.
- `media-generation`: geração e edição de imagem e vídeo pelas ferramentas do Clovy direto na fal.

### Modified Capabilities

- `llm-providers`: seleção por uso sem Clovy API e chat só em CLI; notas sempre direto ao provedor; falha sem chave. Sai o endpoint como modelo do agente.
- `dictation`: transcrição e limpeza sempre direto ao provedor; falha clara sem chave Groq.
- `local-account`: sem token nem endereço do Clovy API; nenhum servidor do Clovy é chamado.

## Impact

- Rust:
  - `src-tauri/src/clovy_api.rs` é apagado ou desmontado; o código direto (Groq, OpenRouter, CLI, endpoint) vai para módulos de provedor.
  - Também mudam `local_identity.rs`, `lib.rs`, `commands.rs`, `providers/mod.rs`, `llm/{registry,mod}.rs`, `domain/processing.rs`, `dictation.rs`, `audio/{capture,live_preview}.rs`, `routines.rs`, `image_safety.rs`, `mcp_server/tools/media.rs`, `computer_use.rs`, `chat_engine/mod.rs` e os testes `clovy_api_*_tests.rs`.
- Frontend:
  - `src/lib/{chat-engine,llm-providers,tauri,errors,agent-model-selection,model-privacy,suggested-models}.ts`;
  - `ChatEnginePicker.tsx`, `AgentWorkspace.tsx`, `NoteChatPanel.tsx`;
  - `settings/{LlmProvidersSection,AppSettings,StyleSettingsSection,ModelPickerDialog,ModelPickerPopover}.tsx`;
  - `share/ShareDialog.tsx`, que sai;
  - i18n.
- Repo:
  - `clovy-api/` é apagado;
  - `.github/workflows/clovy-api.yml`, `coverage.yml`, `dependabot.yml` e `.github/actions/setup-rust`;
  - `Makefile`, `package.json`, `scripts/tauri-dev.mjs`, `scripts/tauri-before-dev.mjs`, `scripts/check-org-references.mjs`;
  - `src-tauri/build.rs`, `src-tauri/clippy.toml`, `.env.example`;
  - `src/test/technical-identity.test.mjs`.
- Docs e regras:
  - `AGENTS.md` e `CLAUDE.md` (regras de fronteira do Clovy API), `CONTEXT.md`;
  - `README.md`, `PRIVACY.md`, `SECURITY.md`, `CONTRIBUTING.md`;
  - `docs/{configuration,llm-providers,dictation,audio-pipeline,development,index}.md`;
  - ADR nova que supera a ADR-0021 e as partes de servidor das ADRs 0065, 0070, 0008, 0013, 0015 e 0062.
- Changes abertas:
  - `cli-substitui-agente-nativo` perde o requisito "Contrato do clovy-api preservado", e a tarefa 4.6 passa a apontar para esta change.
  - `replace-venice-with-groq-openrouter-fal-tavily` (0/20, sobre o servidor) perde o objeto; fica para o dono arquivar ou apagar.
