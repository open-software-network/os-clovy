# Design

## Context

- Clovy é Tauri v2 com core Rust (`src-tauri/`), React (`src/`), sidecar Node do Agents SDK (`agent-runtime/`) e Clovy API (`clovy-api/`, sem mudança aqui). O app continua vivo na barra de menu ao fechar a janela (`src-tauri/src/lib.rs:794-798`) e tem login item (`tauri-plugin-autostart`).
- Banco principal: sqlx/SQLite `notes.sqlite3`, WAL, migrações em catálogo append-only (`src-tauri/src/db/migrations.rs`, versões 1..50). Builds de debug usam diretórios `-dev` (`src-tauri/src/app_paths.rs`).
- Configurações: JSON por domínio em `app_config_dir` (`provider-settings.json`, `dictation-settings.json`, `experimental-settings.json`), estado em `Mutex` gerenciado pelo Tauri; Keychain via `credential_compat.rs` (serviços `co.opensoftware.clovy.*`).
- Modelos: o agente chama `__clovy_model_chat_completions` → `agent_runtime/host.rs` → `clovy_api::proxy_agent_chat_completions`, com rota `AgentGenerationRoute::{Local,Remote}` (`clovy_api.rs:1418-1521`). Já existe um endpoint local OpenAI-compatível único (`LocalGenerationSettings`, `PROVIDER_LOCAL`) usado em notas e agente. Limpeza de ditado e transcrição só vão ao Clovy API.
- Eventos de sessão do agente: `clovy://agent-runtime-event` com métodos `run.started`, `message.delta`, `reasoning.delta`, `tool.*`, `run.completed/failed`; itens persistidos em `agent_items` (`agent_runtime/domain.rs`, `src/lib/agent-runtime-contract.ts`).
- Ferramentas do host centralizadas em `agent_runtime/tools.rs::dispatch_tool`; cliente MCP completo em `agent_mcp.rs`; não há servidor MCP.
- Já há AX (`computer_use_driver.rs`, crate `platform-macos`), ScreenCaptureKit, `tauri-plugin-notification`, deep links de privacidade (`commands.rs:1149-1175`). Não há Vision OCR, SQLCipher nem FTS5.
- Referência de comportamento (somente leitura): Meridian em `/Users/guilhermevarela/Documents/Projetos/SelfHosting/meridian` — captura `tray/src-tauri/src/capture/`, ETL `src/etl/`, categorizador `src/intelligence/session_categorizer/`, ingestão `src/coding_agent_session_ingest/`, destilação `src/worklog_pipeline/distiller/` + `src/embedder/`, hora/frentes `src/worklog_pipeline/{hour,workstream}.rs`, resumo `src/day_summary/`, notificações `meridian-core/src/notifications/`, provedores `src/llm/` + `meridian-core/src/llm_provider.rs`, prompts `src/llm/prompts.rs`, MCP `packages/meridian-mcp/`, cifra `meridian-core/src/db_crypto.rs` + `tray/src-tauri/src/db_key.rs`.

## Goals / Non-Goals

**Goals:**
- Comportamento equivalente ao Meridian nas capacidades da proposta, com código, nomes e UI do Clovy.
- Tudo dentro do processo do Clovy; nenhum daemon separado.
- Banco principal, rotas atuais do Clovy API, Venice BYOK e endpoint local continuam funcionando.

**Non-Goals:**
- Jira, worklog e postagem em trackers; plano diário e aderência; confirmação de troca de tarefa por notificação (alimenta o mapeamento de tickets); login Clerk; telemetria; LLM Lab.
- Windows: a captura e o servidor MCP são macOS; no Windows os recursos ficam ocultos.
- Mudança de contrato do Clovy API.

## Decisions

1. **Processo único.** Captura em thread dedicada; ETL, retenção, ingestão, relatórios e agendador como tarefas `tauri::async_runtime::spawn` iniciadas no `setup` (padrão de `routines::start_scheduler`). Alternativa daemon `launchd` (Meridian) rejeitada: duplica ciclo de vida e permissões TCC. Sono/despertar detectados por salto de relógio, como `meeting_detection.rs`, ou por notificação de workspace quando necessário.
2. **Banco de atividade.** Arquivo `activity.sqlite3` no `app_data_dir`, SQLCipher com chave aleatória de 32 bytes no Keychain serviço `co.opensoftware.clovy.activity-db` (sufixo `-dev` em debug). Como sqlx e qualquer binding usam o mesmo `libsqlite3-sys`, a feature de SQLCipher é unificada para o binário inteiro (Meridian usa `bundled-sqlcipher-vendored-openssl`); o banco principal continua sem `PRAGMA key` e deve abrir igual — isso é regressão obrigatória. Migrações próprias do banco de atividade, no mesmo formato append-only do principal, em módulo separado.
3. **Captura.** Extração AX com habilitação `AXManualAccessibility`/`AXEnhancedUserInterface` para Chromium/Electron, OCR via Apple Vision em memória, exclusões aplicadas antes da escrita. Meio técnico (crates públicos como os do screenpipe fixados por revisão, `objc2`, ou helper Swift no padrão de `src-tauri/native` + `build.rs`) fica a critério do executor, medido e justificado no report. Event tap para eventos de entrada exige Monitoramento de Entrada; sem ela, eventos ficam desligados e a captura de quadros continua.
4. **Configurações.** `activity-settings.json` (captura, monitores, exclusões, horário de trabalho, retenção, vídeo protegido, fontes de agentes, horário do resumo, notificações/horário silencioso, servidor MCP). Provedores estendem `provider-settings.json`: registro de endpoints nomeados (o `localGeneration` atual migra como entrada), seleção por uso (`chat`, `notes`, `dictationCleanup`, `activity`) e níveis de saída estruturada medidos; segredos no Keychain `co.opensoftware.clovy.llm-providers`.
5. **Provedores.** Núcleo Rust `llm` com trait de geração pontual (`prompt` + schema opcional → JSON/texto), implementações CLI (flags equivalentes às do Meridian: `claude -p --no-session-persistence --json-schema`, `codex exec --ephemeral --output-schema -s read-only`, `pi --print --mode json --no-session --no-tools ...`, `agy -p --output-format json --json-schema`, `cursor-agent -p`, `copilot -p`) e HTTP OpenAI-compatível com limite de RPM. PATH resolvido uma vez via shell de login (`$SHELL -lc`), preservando o ambiente real do perfil (lição do Meridian: Pi sem `PI_CODING_AGENT_DIR` falha com HOME alterado). Semáforo de concorrência 1 para chamadas de atividade. Marcador de autoria do Clovy em todo prompt.
6. **Roteamento existente.** Notas, limpeza de ditado e agente consultam a seleção por uso antes do Clovy API, nos pontos já existentes (`clovy_api.rs:409-411`, `488-499`, `1012-1016`). Transcrição de áudio permanece no Clovy API (CLIs não transcrevem).
7. **Motor de chat CLI.** Adaptador Rust por sessão que substitui o sidecar só quando o motor da sessão é um CLI: processo por turno com retomada (`claude --resume <id> --output-format stream-json`, `codex exec resume --json`, `pi --mode json` com arquivo de sessão, modos JSON de `agy`/`cursor-agent`/`copilot`), traduzindo o stream para os métodos e itens existentes, persistindo pelo repositório do agente e reaproveitando cancelamento. Ferramentas do Clovy chegam ao CLI pelo servidor MCP do Clovy (`--mcp-config` ou equivalente). Mensagens durante execução viram mensagem seguinte. Endpoints OpenAI-compatíveis continuam no sidecar como modelo comum.
8. **Servidor MCP.** Binário stdio pequeno empacotado com o app (`clovy-mcp`) que encaminha JSON-RPC para o app por socket Unix em `app_data_dir` (permissão 0600) com segredo da instalação; o app atende reaproveitando consultas existentes e o banco de atividade. Alternativa de abrir os bancos direto no binário rejeitada: exigiria a chave do Keychain fora do app e concorrência com a captura.
9. **Busca.** FTS5 no banco de atividade (SQLCipher inclui FTS5) para títulos, URLs e texto.
10. **Destilação.** Embeddings locais com modelo pequeno baixado uma vez para `app_data_dir/models` (Meridian: candle BERT); runtime a critério do executor dentro da regra de dependências.
11. **UI.** Vista de primeiro nível "Hoje" (`SidebarView` + `workspace-lazy.tsx` + `app-workspace-view.tsx` + `tabMeta`), abas de configurações "Atividade" e seção de provedores na aba Modelos, seção do servidor MCP na aba Agente; ícones `central-icons`, tokens, sentence case, catálogos `defineMessages` en/pt-BR. Gráficos do resumo renderizados com componentes do Clovy; números ligados aos dados gravados.
12. **Recortes e ordem.** S1a provedores núcleo e S2 captura+banco (onda 1, paralelos); S3 linha do tempo e S5 ingestão (onda 2); S4 inteligência do dia e S6 MCP (onda 3); S1b motor de chat CLI (onda 4, depende de S1a e S6). Cada recorte em worktree e branch próprios, integrados no `main` local pelo Main.

## Risks / Trade-offs

- [SQLCipher unificado muda a biblioteca SQLite do app inteiro] → suíte de migrações e testes do banco principal obrigatória no S2; build de produção verificado.
- [Permissões TCC do binário de debug diferem do app assinado] → prova funcional no build de desenvolvimento com permissões concedidas a ele; sem permissão a prova fica bloqueada, não simulada.
- [CLIs mudam flags e formatos de stream] → adaptadores isolados por CLI, testes com CLIs falsos, versão detectada exibida.
- [CLI no chat perde ferramentas nativas e steer] → MCP do Clovy fornece ferramentas; steer vira mensagem seguinte; decisão do dono registrada.
- [Volume de escrita a cada 2 s] → banco separado, retenção com vácuo incremental, pausa por pouco disco.
- [Ações em notificação dependem do plugin] → se `tauri-plugin-notification` não suportar ações no macOS, o clique abre a vista e "Adiar 1h" fica na própria vista; registrar no report.
- [Conflitos de integração entre recortes paralelos] em registradores compartilhados (`lib.rs`, `commands.rs`, `settings-config.ts`, catálogos i18n, `Sidebar.tsx`) → Main integra em ordem e resolve.
