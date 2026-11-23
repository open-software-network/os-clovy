# Proposal

## Why

O dono usa o Clovy como app principal e quer, dentro dele e com implementação própria, o que hoje só o Meridian faz: saber o que fez no dia a partir da tela e dos agentes de código, revisar uma linha do tempo, receber resumo e standup prontos e usar seus próprios provedores de IA (CLIs locais e endpoints OpenAI-compatíveis) em todo o app. O Meridian (`/Users/guilhermevarela/Documents/Projetos/SelfHosting/meridian`) é a referência de comportamento; nenhum código dele roda no Clovy e o Clovy não lê os dados do Meridian.

## What Changes

- **Provedores de LLM:** registro de provedores no Clovy com CLIs locais (`claude`, `codex`, `pi`, `agy`, `cursor-agent`, `copilot`) e vários endpoints OpenAI-compatíveis nomeados; detecção, teste de conexão e probe de saída estruturada. Valem para geração pontual (notas, limpeza de ditado, recursos de atividade) e como motor do chat/agente, inclusive CLIs, com o Clovy API continuando como opção.
- **Captura de atividade:** captura contínua só de texto (árvore de acessibilidade com fallback para OCR do Apple Vision), a cada 2 s na tela principal e ~10 s em monitores secundários, eventos de entrada sem conteúdo digitado, clipboard redigido, exclusões (apps, domínios, aba anônima, vídeo protegido, horário de trabalho), retenção de 30 dias, pausa manual.
- **Banco de atividade cifrado:** banco SQLite separado com SQLCipher e chave no Keychain; o banco principal do Clovy não muda.
- **Linha do tempo:** ETL de quadros para sessões por app, lacunas de ocioso/suspensão, categorização determinística em 10 classes, nova vista "Hoje"/linha do tempo na UI do Clovy, estatísticas e busca no histórico.
- **Sessões de agentes de código:** ingestão de transcrições locais do Claude Code, Codex, Copilot CLI/VS Code, Cursor/Cursor CLI e Antigravity, com resumo pelo próprio CLI e proteção contra autoingestão.
- **Inteligência do dia:** destilação local (incluindo deduplicação semântica com embeddings locais), relatório por hora, frentes de trabalho do dia, resumo diário com insights, standup e painéis, geração automática no horário configurado com recuperação após suspensão, e notificações (horário silencioso, ações).
- **Servidor MCP do Clovy:** servidor MCP local que expõe notas, transcrições, ditados, memórias e os dados de atividade (sessões, timeline, estatísticas, busca, sessão ativa) para clientes como Claude Code e Cursor.
- **Fora do escopo (decisão do dono):** Jira e qualquer worklog/postagem em trackers (Linear, GitHub, Trello, Azure DevOps, Asana), plano diário/aderência a tickets, login Clerk, telemetria PostHog/OpenObserve, LLM Lab.
- Privacidade: texto da tela e de transcrições de agentes só vai ao provedor que o usuário escolher para atividade; sem escolha, os recursos de IA sobre atividade ficam desligados, sem fallback para o Clovy API.

## Capabilities

### New Capabilities
- `llm-providers`: registro, detecção, teste e roteamento de provedores (CLIs e endpoints OpenAI-compatíveis) para geração pontual e como motor de chat.
- `activity-capture`: captura de texto da tela e eventos, controles de privacidade, retenção e banco de atividade cifrado.
- `activity-timeline`: sessões, lacunas, categorização, vista de linha do tempo, estatísticas e busca no histórico.
- `coding-agent-ingest`: ingestão e resumo de sessões de agentes de código locais.
- `day-intelligence`: destilação, relatórios por hora, frentes de trabalho, resumo do dia, standup, agendamento e notificações.
- `clovy-mcp-server`: servidor MCP local com dados do Clovy e da atividade.

### Modified Capabilities
- Nenhuma em `openspec/specs/` (OpenSpec recém-iniciado). Comportamento existente afetado: o seletor de modelo e as rotas de geração do Clovy (`src-tauri/src/clovy_api.rs`, `src-tauri/src/providers/mod.rs`) passam a considerar os novos provedores, preservando Clovy API, Venice BYOK e o endpoint local atual.

## Impact

- Rust (`src-tauri/`): novos módulos de provedores, captura (FFI AX/Vision/CoreGraphics), ETL, ingestão, inteligência do dia, servidor MCP; novo banco cifrado; novas dependências (SQLCipher, embeddings locais) sob `spec/package-install-security.md`.
- Frontend (`src/`): vista de linha do tempo/hoje, abas de configurações (Atividade/Privacidade, Provedores), onboarding de permissões, i18n en + pt-BR, regras de `spec/`.
- Sidecar (`agent-runtime/`) e host do agente: adaptador de sessão para motores CLI e endpoints.
- Permissões macOS: Acessibilidade, Gravação de Tela, Monitoramento de Entrada, Notificações.
- Clovy API (`clovy-api/`): sem mudança de contrato.
