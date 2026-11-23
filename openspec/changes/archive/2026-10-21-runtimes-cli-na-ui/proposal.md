# Proposal

## Why

Com o chat rodando em Claude Code e Codex (change `cli-substitui-agente-nativo`), a interface ainda carrega sobras do agente nativo e dependências de fora que o dono não quer (feedback de 2026-10-18 sobre a tela inicial, Configurações e Plugins):

- O rodapé do início ("O Clovy roda localmente. Você está usando {modelo}") e o aviso do composer ("{CLI} executa neste Mac com as ferramentas do Clovy") ocupam espaço sem ajudar.
- Claude Code, Codex e os outros CLIs aparecem só por nome ou com ícone genérico; a marca real deles não aparece em lugar nenhum.
- O conector GitHub só conecta pelo device flow da GitHub App do Clovy, mesmo quando o usuário já está logado no `gh` neste Mac.
- O uso do computador exige plano Pro/Max da conta Clovy, um modelo de visão do catálogo da Clovy API para ficar "Pronto" e a resposta do interruptor remoto da Clovy API, embora nos CLIs a captura vá ao modelo do próprio CLI pelo `clovy-mcp`.

## What Changes

- Sai o rodapé de privacidade do início do chat e sai o aviso de motor do composer e do seletor de motor.
- Ícones de marca reais (SVG) para cada runtime e provedor exibidos na interface: Claude Code, Codex, GitHub Copilot, Cursor Agent, Antigravity, Pi, OpenRouter e os já existentes; monograma só quando a marca não tem SVG de licença aberta.
- GitHub pelo `gh`: se o `gh` estiver instalado e logado em `github.com` e não houver conta GitHub conectada, o Clovy conecta sozinho como "GitHub (via gh)", lendo o token do `gh` a cada chamada, sem copiá-lo. Os pacotes de leitura e escrita do Clovy continuam limitando as ferramentas. Desconectar essa conta impede a reconexão automática.
- Uso do computador sem dependência externa: some o requisito de plano, de modelo de visão da Clovy e do interruptor remoto; "Pronto" depende só do driver local, das permissões do macOS e da ativação do usuário. A ferramenta continua só pelo `clovy-mcp`, com aprovação por ação (ADR-0028).
- **BREAKING** O agente nativo deixa de poder usar o uso do computador (ele dependia do modelo de visão da Clovy); a ferramenta fica só nos turnos de CLI com MCP.

## Capabilities

### New Capabilities

- `chat-engine-presentation`: o que a interface mostra sobre o motor do chat (sem rodapé, sem aviso) e a identidade visual dos runtimes e provedores.
- `github-cli-connection`: conexão automática do conector GitHub pela sessão do `gh`.
- `computer-use-availability`: quando o uso do computador está disponível e de quem depende.

### Modified Capabilities

(nenhuma: não há spec canônica dessas áreas em `openspec/specs/`.)

## Non-goals

- Configurações > Modelos (escolha de modelo e esforço por CLI, seletor OpenRouter de texto): o dono pediu para não mexer agora.
- Remover o agente nativo (fase 4 da `cli-substitui-agente-nativo`).
- Registrar o MCP do driver cua direto no Claude/Codex (rejeitado pela ADR-0028).
- Mudar rotas da Clovy API (ADR-0021).

## Impact

- Frontend: `src/components/agent/AgentWorkspace.tsx`, `composer/ChatEnginePicker.tsx`, `composer/ModelPicker.tsx` (`heroPrivacyFootnote`), `ProviderLogo.tsx`, `settings/{LlmProvidersSection,McpServerSection,CodingAgentSourcesSection,ConnectorsSection}.tsx`, `plugins/ComputerUseControl.tsx`, i18n, novos SVGs em `src/assets/`.
- Rust: `src-tauri/src/connectors/**` (fonte de token `gh`), `src-tauri/src/shell_env.rs` (uso), `src-tauri/src/computer_use.rs` (gates de plano, visão e rollout).
- Docs: ADR nova para o GitHub via `gh` (adendo à ADR-0036) e ADR nova para o uso do computador sem gates externos (adendo à ADR-0028); `AGENTS.md` se alguma regra de credencial mudar.
