# Design

## Context

Mapa do código em `main` `81d3d3b1` (scouts de 2026-10-18):

- **Rodapé do início**: `AgentWorkspace.tsx:2544-2548`, ramo `heroMode` (`:711`), texto de `heroPrivacyFootnote` (`composer/ModelPicker.tsx:886-901`), chaves `chat.privacy.{running,e2ee,private,anonymous,runsLocally}` (`src/i18n/messages/chat.ts:183-187,392-397`). Teste que fixa texto: `src/test/i18n-chat.test.tsx:102-106` (ramo `private`).
- **Aviso de motor**: `ChatEngineNotice` (`composer/ChatEnginePicker.tsx:88-127`), usado no composer (`AgentWorkspace.tsx:3194`) e dentro do menu do seletor quando `showNotice` (`ChatEnginePicker.tsx:365`); chaves `chat.engine.notice.*` (`chat.ts:365-369`); testes `src/test/chat-engine-picker.test.tsx:243-275`.
- **Identidade visual**: `ProviderLogo.tsx:21-59` usa `central-icons` (Claude, OpenAI, Gemini…) e monograma; não há Codex, Copilot, Cursor, Antigravity, Pi nem OpenRouter. Sem ícone de marca: seletor de motor (`ChatEnginePicker.tsx:272-386`), `LlmProvidersSection.tsx:416-428,522`, `McpServerSection.tsx:105-122`, `CodingAgentSourcesSection.tsx:78-84`, `CodingAgentLane.tsx:57-78`, `ImportClaudeProjectsDialog.tsx:95-98` (ícone genérico), `AgentActionCards.tsx:178-206`. CLIs do registro: `claude`, `codex`, `pi`, `agy`, `cursor-agent`, `copilot` (`src-tauri/src/llm/cli.rs:27-72`); fontes de agentes de código em `coding-agents.ts:50-58`.
- **GitHub**: device flow da GitHub App direto do desktop (`connectors/github.rs`, `connectors/mod.rs:1602-1754`), token no Keychain `co.opensoftware.clovy(-dev).github` (`store.rs:20-28`), checagem de instalação da App no connect (`github.rs:548-572`), pacotes `github_read`/`github_write` (`policy.rs`), ferramentas pelo mesmo dispatch no nativo e no `clovy-mcp` (`native_connectors.rs:135-283,546-586`, `mcp_server/tools/connectors.rs:304-347`). PATH do shell de login: `shell_env.rs` (`LoginEnv::which`).
- **Uso do computador**: plano `plan_eligible` (`computer_use.rs:2011-2029`), visão `generation_model_supports_vision` (`:3139-3151`, `providers/mod.rs:693-700`), status `status_inner` (`:1822-1991`, inclui rollout em `:1844-1858`), rollout `rollout_gate` (`:1708-1773`, `clovy_api.rs:1089-1099`), grant `set_computer_use_grant` (`:2228-2248`). Turno CLI já sem plano/visão (`:3057-3065`); ferramenta nativa em `agent_runtime/tools.rs:213-226`.

## Goals / Non-Goals

**Goals:** requisitos das três specs desta change. **Non-Goals:** ver `proposal.md` (Configurações > Modelos fica para depois, por decisão do dono).

## Decisions

### D1. Remoção limpa do rodapé e do aviso
Apagar o render, `heroPrivacyFootnote`, `ChatEngineNotice`, a prop `showNotice` e as chaves i18n que ficarem sem uso. O selo de privacidade do modelo (`ModelPrivacyChip`, `model-privacy.ts`) fica, pois é usado em outros lugares. O teste que fixava a frase `private` é apagado (fixava texto, não comportamento). O campo `clovy_tools` do backend fica se ainda tiver outro consumidor; senão sai junto.

### D2. Ícones: um componente de logo único com SVGs vendorizados
Estender `ProviderLogo` (ou um `RuntimeLogo` ao lado, reaproveitando o monograma) com SVGs em `src/assets/brand/`, de fonte com licença aberta registrada (preferência: `lobe-icons`, MIT; alternativa `simple-icons`, CC0), com o arquivo `src/assets/brand/LICENSES.md` listando origem e licença de cada um. Ids cobertos: `claude`, `codex`, `copilot`, `cursor-agent`, `agy`, `pi`, `openrouter`, mais os provedores já cobertos. `<select>` nativos não mostram ícone: o ícone vai no título da linha/cartão ao lado do select. Seguir `~/.agents/skills/impeccable/SKILL.md` para tamanho, alinhamento e modo escuro.

### D3. GitHub via gh como segunda fonte de credencial
Nova variante de conta `github_cli` (mesmo provedor `github`, mesma tabela de contas e pacotes). Detecção: `LoginEnv::which("gh")`, `gh auth status --hostname github.com` (código de saída), identidade por `GET /user` com o token de `gh auth token --hostname github.com`. Token nunca persistido: uma função `github_access_token(account)` devolve o token do Keychain para contas da App e o do `gh` (processo filho, stdout capturado em memória, zeroize) para `github_cli`; todo call site de `github.rs` passa por ela. Sem checagem de instalação da App para `github_cli`. Pacote `github_read` ativo na criação, `github_write` desligado (o usuário liga em "Adicionar acesso"). Detecção roda no início do app e ao abrir Plugins, com timeout curto (2 s) e sem bloquear a UI. Preferência `githubCliAutoConnect` (padrão ligado) fica falsa ao desconectar a conta via `gh`; botão "Conectar pelo GitHub CLI" a religa e conecta. Revoke não se aplica (o token é do `gh`). Erros do `gh` não expõem o token (stderr filtrado). ADR nova: adendo à ADR-0036.

Alternativa rejeitada: copiar o token do `gh` para o Keychain do Clovy. Duplica o segredo, envelhece sem renovação e diverge do `gh` após logout.

### D4. Uso do computador só local
Remover de `status_inner` e de `set_computer_use_grant` as checagens de plano, de modelo de visão e de rollout; os estados `pro`, `needs_vision` e `rollout_disabled` saem do status e da UI (`ComputerUseControl.tsx:29-42`). `rollout_gate`, `computer_use_rollout` no desktop e o cache saem se não tiverem outro uso (a rota da Clovy API fica, ADR-0021). A ferramenta sai do catálogo nativo (`agent_runtime/tools.rs`), já que o nativo dependia do modelo de visão; o caminho `ActionOrigin::NativeAgent` fica inalcançável e sai junto se não tiver outro chamador. Texto do privacy tip passa a dizer que as capturas vão ao CLI do turno. ADR nova: adendo à ADR-0028 (gates de plano/rollout/visão retirados; broker, lease e aprovação por ação mantidos).

### D5. Sem mudança de modelos
Nada em Configurações > Modelos, `llm_usage`, `generation_model` ou seletor OpenRouter (decisão do dono, 2026-10-18).

## Risks / Trade-offs

- [Conexão sem clique do GitHub] → só cria conta com leitura; escrita exige ação explícita e aprovação; desconectar desliga a automação.
- [Token do `gh` com escopos amplos] → o Clovy limita pelas suas ferramentas e pacotes; o token não sai do processo nem vai a CLI de agente.
- [Latência de chamar `gh` a cada request] → um processo curto por chamada; aceitável para as poucas chamadas de ferramenta [INFERENCE: ~20-50 ms]. Se medir mais, cache em memória por no máximo 60 s, nunca em disco.
- [Uso do computador sem kill switch remoto] → o fork não tem controle de rollout de produção; o usuário liga/desliga localmente e cada ação pede aprovação.
- [Marca de terceiros] → só SVGs de fontes com licença aberta e registro de origem.

## Migration Plan

Sem migração de dados: contas, grant e preferências existentes continuam. Estados de status removidos deixam de ser emitidos; a UI não depende mais deles. Rollback: reverter o merge.
