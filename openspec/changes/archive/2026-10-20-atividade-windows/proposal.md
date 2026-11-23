# Proposal

## Why

O Clovy para Windows já é distribuído, mas a captura de atividade só existe no macOS. No Windows a tela "Hoje" fica vazia, a aba Atividade some dos ajustes, as sessões de agentes de código não são lidas e as frentes de trabalho nunca são geradas. O motor, a linha do tempo e a inteligência do dia já são independentes do sistema; falta o adaptador do Windows e destravar o banco cifrado nesse alvo.

## What Changes

- Novo adaptador de captura para Windows com a mesma cadência e as mesmas regras do macOS: janela em foco (app, título, URL do navegador), texto pela UI Automation, OCR local do Windows quando o texto vem fraco, contagem de cliques e teclas sem conteúdo, área de transferência redigida, tela secundária, espaço em disco e tempo desde a última entrada do sistema.
- Nenhuma imagem é gravada, como no macOS.
- O banco de atividade passa a ser cifrado com SQLCipher também no Windows, com a chave no Gerenciador de Credenciais.
- A captura opera no Windows sem pedir permissões do sistema (o Windows não tem equivalente); a interface não mostra os pedidos de permissão do macOS nesse sistema.
- Reconhecimento do Windows: navegadores, editores e apps de tela bloqueada (`LockApp`, `LogonUI`) por nome do executável; janelas InPrivate e anônimas nunca passam por OCR.
- Leitura das sessões de agentes de código no Windows, incluindo os caminhos de VS Code e Cursor em `%APPDATA%`.
- Embeddings locais das frentes também no Windows (CPU), para a mesma qualidade de agrupamento.
- Ajustes: aba Atividade visível no Windows; vista "Hoje" com mensagem própria quando a captura não é suportada (Linux).
- Ícones dos apps em "Principais apps" extraídos do executável.

## Capabilities

### New Capabilities

Nenhuma.

### Modified Capabilities

- `activity-capture`: permissões e OCR deixam de ser específicos do macOS; banco cifrado com cofre de credenciais do sistema; novo requisito de captura no Windows.
- `activity-timeline`: tela bloqueada do Windows não conta como trabalho.
- `coding-agent-ingest`: locais padrão das fontes no Windows.

As capacidades canônicas já estão estabelecidas em `openspec/specs/` após a integração e o arquivamento de `add-activity-intelligence` e `atividade-ocioso-e-minutos` na branch principal.

## Impact

- Rust: novo módulo `src-tauri/src/activity/windows/`; `activity/mod.rs` (setup no Windows), `activity/filter.rs`, `activity/timeline/builder.rs`, `timeline/context.rs`, `timeline/categorize.rs` (nomes do Windows), `activity/app_icon.rs`, `src-tauri/src/coding_agents/sources/`, `src-tauri/src/day_intelligence/embedder.rs`.
- Dependências: features novas do crate `windows` (UI Automation, captura de tela, OCR, entrada bruta, área de transferência), `libsqlite3-sys` com SQLCipher no alvo Windows (OpenSSL compilado, exige Perl e NASM no build), candle sem Accelerate no alvo Windows.
- Build e CI: `scripts/build-windows.ps1` e o job `windows-rust` de `.github/workflows/desktop.yml` passam a preparar Perl e NASM e a rodar os testes do módulo de atividade.
- Frontend: `src/components/settings/settings-config.ts`, `AppSettings.tsx`, `ActivitySettingsSection.tsx`, `src/components/activity-timeline/ActivityTimelineView.tsx`, mensagens de atividade.
- Docs: `docs/activity-capture.md`, `docs/activity-timeline.md`, `docs/coding-agent-sessions.md`, `docs/day-intelligence.md`, `README.md` (lista do que o Windows cobre).
- Prova em máquina Windows real (`cert`, via Tailscale e SSH).
