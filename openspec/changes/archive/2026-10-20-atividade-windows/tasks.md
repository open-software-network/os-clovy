# Tasks

## 1. Base no Windows: build, banco cifrado e runtime

- [x] 1.1 Preparar o `cert` (perfil do usuário `cert-`): rustup `stable-x86_64-pc-windows-msvc`, Strawberry Perl, NASM, pnpm via corepack, clone do repositório; registrar versões em `.tmp/verify/atividade-windows/cert-setup.md`.
- [x] 1.2 RED: teste de integração do store que abre o banco cifrado e confirma que SQLite comum não lê o arquivo, rodando no Windows; verificar que falha no `cert` com `CipherUnavailable`.
- [x] 1.3 GREEN: SQLCipher vendored no alvo Windows (`Cargo.toml`), checagem de Perl/NASM em `scripts/build-windows.ps1`, instalação no job `windows-rust` de `.github/workflows/desktop.yml` com `cargo test` do módulo `activity`; teste de 1.2 passa no `cert`. Sintaxe YAML/PowerShell validada; `workflow_dispatch` pendente da integração pelo dono, conforme exceção deste mandato.
- [x] 1.4 RED: teste de setup que espera runtime de atividade no Windows com plataforma falsa e chave em cofre simulado; verificar que falha.
- [x] 1.5 GREEN: esqueleto `activity/windows/` com `WinActivityPlatform` (permissões `Granted`, demais métodos devolvendo vazio), `setup`/`start` no Windows com `KeychainKeyStore`; teste de 1.4 passa e o app de desenvolvimento no `cert` mostra captura ligada sem quadros.

## 2. Janela, identidade e listas

- [x] 2.1 RED: testes unitários de identidade (nome do executável em minúsculas, `FileDescription`, AppUserModelID) e das listas Windows em `filter.rs`, `timeline/context.rs`, `timeline/categorize.rs` e `timeline/builder.rs` (`msedge.exe` é navegador, `code.exe` é editor, `lockapp.exe` nunca é útil); verificar que falham.
- [x] 2.2 GREEN: `focused_window` (D2) e listas; testes de 2.1 passam; no `cert`, quadros mostram app e título corretos no export de depuração.

## 3. Texto, URL e OCR

- [x] 3.1 RED: testes do extrator UIA com árvore simulada (orçamento de nós e tempo, URL do controle de endereço, janela privada); verificar que falham.
- [x] 3.2 GREEN: `accessibility_text` e URL por UI Automation (D3); testes de 3.1 passam; no `cert`, quadro do Edge com URL e texto e origem "acessibilidade".
  - Ajuste D3 autorizado em 2026-10-19: metadados da URL confirmada do documento, sem esquema inventado ou endereço em edição. RED `8a21ee8a`: cinco falhas; GREEN `a04d486d`: 16 testes UIA passaram no cert. Prova nativa final: quadros 170–172 com `http://127.0.0.1:19362/`, marcador visível e origem `accessibility`; URL e texto vistos no detalhe de sessão em "Hoje". Domínio excluído mais endereço não submetido: zero quadros novos; senha sintética ausente dos exports. Recibos e capturas em `.tmp/verify/atividade-windows/`.
- [x] 3.3 RED: teste de integração do OCR com imagem de teste reconhecida pelo `Windows.Media.Ocr`, rodando no `cert`; verificar que falha.
- [x] 3.4 GREEN: `ocr_text` com Windows Graphics Capture e `Windows.Media.Ocr` (D4), sem arquivo em disco; teste de 3.3 passa; no `cert`, janela sem árvore UIA gera quadro com origem "OCR" e nenhum arquivo de imagem novo.

## 4. Entrada, área de transferência, telas, disco e ociosidade

- [x] 4.1 RED: testes de normalização do Raw Input (só instantes de mouse down e key down, sem código de tecla) e do filtro de formatos ocultos da área de transferência; verificar que falham. RED adicional de `system_idle_ms` executado no `cert` após a integração da dependência.
- [x] 4.2 GREEN: `set_input_capture`/`drain_input_events` (D5), `clipboard_text_if_changed`, `secondary_windows`, `free_disk_bytes` (D6) e `system_idle_ms`; testes de 4.1 passam; no `cert`, cliques, teclas e cópia aparecem em `input_events` sem conteúdo digitado.
  - Dependência integrada pelo merge `2264cae6`. RED `aada6371`: 0 passaram / 1 falhou pela asserção esperada. GREEN `b0901c92`: quatro testes Windows, um de persistência comum e um de tela bloqueada passaram. `GetLastInputInfo` + `GetTickCount` usa `wrapping_sub` em `u32`; a coluna e a migração 6 vêm do main, sem segunda migração. Export nativo: quadro 234 com `inputIdleMs = 1866907` enquanto Eventos de entrada estava desligado. Evidência em `.tmp/verify/atividade-windows/round3/`.
  - Prova curta nativa: `click` e `key` com contagem 1 e sem conteúdo; uma cópia redigida; marcadores de clipboard oculto e de texto digitado ausentes do export. Seleção secundária corrigida após RED: parar na janela focada/própria ou em identidade desconhecida; 10 testes passaram. Sem prova física multimonitor.
- [x] 4.3 Atualizar `docs/activity-capture.md` e `docs/activity-timeline.md` com o adaptador Windows, o banco cifrado no Windows e os nomes de tela bloqueada; conferir com o código.

## 5. Agentes de código e embeddings

- [x] 5.1 RED: testes de `SourceRoots` no Windows (perfil e `%APPDATA%`) e da recusa de ponto de reparo ao abrir arquivos; verificar que falham.
- [x] 5.2 GREEN: raízes e abertura segura no Windows (D10); testes de 5.1 passam; no `cert`, uma sessão real de Claude Code ou Codex aparece na faixa de agentes. Atualizar `docs/coding-agent-sessions.md`.
- [x] 5.3 Candle e tokenizers no alvo Windows (D9); teste do embedder BGE passa no `cert`; tempo de embedding de uma hora medido e registrado. Se inaceitável, parar e voltar ao dono. Atualizar `docs/day-intelligence.md`.
  - RED do teste existente com o modelo real observado no `cert`: preparo chegou a `Ready`, mas `embed` retornou `None` e a asserção `vectors` falhou; 1 falha, 13,39 s de teste. Log `5.3-bge-real-model-red.log`.
  - GREEN real e benchmark debug no i5-10300H: 135 textos candidatos de 1.800 amostras sintéticas de uma hora, um batch de embeddings em 55,9095 s e destilação total em 56,7775 s. Verificação/prontidão do modelo separada: 10,13 s; geração da fixture: 1,13 ms. Não é prova de captura ao vivo nem relatório gerado por LLM.

## 6. Interface

- [x] 6.1 RED: testes de componente para a aba Atividade visível com `supported`, seção sem linhas de permissão fora do macOS, vista "Hoje" com estado "não suportado" e ícone de app no Windows; verificar que falham.
- [x] 6.2 GREEN: `settings-config.ts`, `AppSettings.tsx`, `ActivitySettingsSection.tsx`, `ActivityTimelineView.tsx`, mensagens e `app_icon.rs` no Windows (D11), seguindo `/impeccable`; testes de 6.1 passam. Atualizar a lista do que o Windows cobre no `README.md`.
  - Regressão de componente EN/PT para todo o painel Atividade, agentes e agendamento: texto neutro, sem referência a Mac. Confirmado no WebView nativo do Windows; detector Impeccable sem achados nos seis arquivos, limitado à análise regex.

## 7. Verificação

- [x] 7.1 `make verify` verde no Mac (sem mudança de comportamento no macOS) e `cargo test` do módulo `activity`, `coding_agents` e `day_intelligence` verde no `cert`; contagens em `.tmp/verify/atividade-windows/`.
  - Após merge de `origin/main` `4efe2b2e` e GREEN final `f31204ce`: Mac `make verify` passou em 476,74 s, 202 suítes / 2.046 testes frontend; desktop Rust 2.180, companions 33 e Clovy API 599 (agregado 2.812 passaram / 21 ignorados). Windows, sem `--lib`: activity 191 (incluindo três integrações e testes nativos antes ignorados), coding_agents 53, day_intelligence 61 / um ignorado; zero falhas. Probe filho isolado de SourceRoots: um passou. Cadeia Windows com rebuild: 405,2265 s; 75 arquivos de produto alterados desde a base conferidos por SHA-256 contra `f31204ce`, zero divergências. Logs em `round3/`.
- [x] 7.2 Prova ponta a ponta no `cert` com o app de desenvolvimento na sessão de console: captura ligada por 2 horas de uso real, "Hoje" com Focado/Ocioso/Ausente, categorias e principais apps com ícones, frentes geradas para as horas fechadas, faixa de agentes, janela InPrivate descartada, Windows bloqueado fora do foco, banco ilegível sem chave; capturas de tela e export em `.tmp/verify/atividade-windows/`.
  - Rodada pós-merge: coleta real de 7.200,032 s concluída, mesmo PID/sessão, 121 amostras WTS bloqueadas; 1.793 quadros por hora, 3.586 com `input_idle_ms`, zero lacunas de captura acima de quatro segundos. DTO da janela exata: Focado 0 / Ocioso 0 / Ausente 7.200.000 ms; SQLite comum abriu e retornou `SQLITE_NOTADB 26` ao ler o esquema sem chave. Screenshots nativos, exports e métricas em `round3/`. Fechada pelo dono com estes limites: o console ficou bloqueado durante todo o ensaio, então duas horas de uso ativo, InPrivate nativo, categorias/apps com ícones e nova sessão de agente não foram provados no `cert`; sem provedor, zero relatórios/frentes gerados e nenhuma chamada paga. Esses critérios não foram substituídos por fixtures. A rodada anterior provou no `cert` captura com app, título, texto e URL, "Hoje" com domínio e ícone, exclusão de domínio, OCR, clipboard redigido e ingestão de sessão Codex.
- [x] 7.3 Limpar o `cert`: parar o app, remover a tarefa agendada e o clone; manter as ferramentas só se o dono quiser.
  - Removidos app PID 6596 e descendentes, Vite, 13 tarefas próprias, perfis de prova, clone/target/caches locais, transcrição própria de hash conferido, Rust stable 1.99.1 (default restaurado para nenhum), NASM, Perl, helper de ambiente, shims pnpm e sfw. Pós-checagem `2026-10-20T05:28:59Z`: zero processos/tarefas/perfis próprios, clone ausente; C: 3.008.970.752 bytes antes da remoção e 56.779.231.232 bytes livres na observação final. Preservados Node/Git/VS/rustup/Codex/Claude preexistentes, autenticação/credenciais compartilhadas e caches globais; nenhuma toolchain adicionada foi retida. Evidências copiadas e SHA-256 conferido no Mac antes da exclusão. Túnel SSH local encerrado.

## Workflow follow-up

- `add-activity-intelligence` e `atividade-ocioso-e-minutos` já foram mergeadas e arquivadas no main `4efe2b2e`, incorporado pelo merge `2264cae6`. `openspec validate atividade-windows --strict` passou com as canônicas atuais. Esta change permanece aberta, sem merge na main ou arquivamento por este mandato.
- Revisão de código, aceite do dono, arquivamento, `/sync` e `/cleanup`.
