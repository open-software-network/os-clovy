# Tasks

## 1. Resolução no Windows

- [x] 1.1 Reproduzir no `cert` antes de mudar: app de desenvolvimento da `main` atual mostra `claude` e `codex` como não instalados; registrar `where.exe claude codex`, `PATHEXT` e as chaves de PATH do ambiente do app em `.tmp/verify/cli-agentes-windows/`.
  - App de prova `co.opensoftware.june.cliproof` em `f97fa1a9` (código igual à `main`): os seis CLIs "Não encontrado no PATH da sua sessão" (`1.1-models-before.png`). Sessão de console: chave `Path` (não `PATH`), sem `HOME` e sem `SHELL`, `PATHEXT=.COM;.EXE;.BAT;.CMD;...`; `where.exe` acha `.local\bin\claude.exe`, `.codex\bin\codex.exe`, `npm\codex` e `npm\codex.cmd` (`1.1-console-env.txt`).
- [x] 1.2 RED: testes unitários de `LoginEnv` para chave `Path`, `PATHEXT` (escolhe `codex.cmd`, ignora `codex` sem extensão), `PATHEXT` ausente e diretórios de reserva do Windows a partir de `USERPROFILE`/`APPDATA`, com a escrita no mesmo nome de chave; verificar que falham.
  - RED `442272f2` no `cert`: `llm` 50 ok / 4 falhas (`Path`, `PATHEXT`, sem `PATHEXT`, reserva do perfil, e o teste existente `llm_non_executable_file_is_not_a_cli`, que já falhava no Windows).
- [x] 1.3 GREEN: D1–D4 em `shell_env.rs`, com a origem de cada diretório de reserva registrada em comentário; testes de 1.2 passam; testes Unix existentes de `llm` sem mudança.
  - GREEN `4b520560`: `llm` 54 ok / 0 falhas no `cert`; Mac 61 ok. Ajuste do D3: do `PATHEXT` só entram `.com`, `.exe`, `.bat`, `.cmd` (o Rust não inicia `.js`/`.vbs`). D4 com fontes nos instaladores (`claude.ai/install.ps1`, `chatgpt.com/codex/install.ps1`, `antigravity.google/cli/install.ps1`, `cursor.com/install?win32=true`, docs do Copilot/winget) e `%USERPROFILE%\.codex\bin` visto no `cert`.

## 2. Execução íntegra

- [x] 2.1 Provar no `cert` com `.cmd` falso o que chega por argumento e pela entrada padrão para prompts com várias linhas, aspas, `%`, `^`, `&` e acima de 8191 caracteres, pelos caminhos reais `llm::process` e `chat_engine::turn`.
  - Sonda no `cert` com shim npm real (mesmo modelo de `npm\codex.cmd`): por argumento, várias linhas falham com `batch file arguments are invalid` e >8191 caracteres com `Linha de comando muito longa`; aspas, `%`, `^`, `&` chegam iguais; por stdin tudo chega byte a byte (`probe/output.txt`). Caminhos reais: `llm::process` (stdin, 20 mil caracteres) passa já no RED; `chat_engine::turn` falha no RED.
- [x] 2.2 RED/GREEN do D5 se 2.1 mostrar texto alterado ou erro; senão, registrar a prova e manter um teste de integração Windows que trava esse comportamento.
  - D5: com lançador `.cmd`/`.bat`, cursor-agent (lê stdin quando `-p` não tem prompt, conferido no pacote 2026.10.19) e copilot (doc oficial: prompt por pipe) recebem o prompt pela entrada padrão (`chat_engine::cli::prompt_on_stdin`). RED `442272f2`: `chat_engine` 2 falhas; GREEN `4b520560`: 25 ok / 0 falhas no `cert`. Teste-trava do stdin em `llm::process`.

## 3. Documentação

- [x] 3.1 Atualizar `docs/llm-providers.md` com a resolução no Windows e conferir com o código.
  - `7a6dbf30`: seção "Windows resolution", nota no motor de chat e verificação.

- [x] 3.2 Extra pedido pelo dono antes do merge: CI e texto de CLI não instalado.
  - CI `6e064d49`: o job `windows-rust` roda `cargo test ... llm` e `chat_engine` após "Test Windows activity capture"; actionlint sem achado novo (o único é o SC2129 preexistente na linha 141, com os rótulos `blacksmith-*` ignorados).
  - Texto: RED `eff45f04` (o seletor mostrava o `reason` cru do Rust, "`pi` was not found in your login shell PATH."); GREEN `b46967cc`: o seletor usa a chave `chat.engine.notFoundOnPath` (EN "Not found on your PATH.", PT "Não encontrado no PATH da sua sessão."), `settingsPanels.llm.notInstalledReason` em EN fica neutro e o motivo de `llm/detect.rs` não cita mais shell de login. A chave `chat.engine.notInstalled` saiu, porque ficou sem uso. Testes do seletor (EN e PT) e de Ajustes verdes; o detector do impeccable não achou nada.

## 4. Verificação

- [x] 4.1 `make verify` verde no Mac; `cargo test --manifest-path src-tauri/Cargo.toml --locked llm` e `chat_engine` verdes no `cert`.
  - Mac `make -k verify`: vitest 202 arquivos / 2046 testes ok, typecheck ok, fmt/clippy ok, Rust 2812 ok / 0 falhas; Biome acusa 2 erros só em `.mcp.json` e `opencode.json` do working tree (mudança de ferramenta não commitada); as versões commitadas passam. `cert` (`7a6dbf30`/`4b520560`): `llm` 54 ok, `chat_engine` 25 ok, clippy Windows sem aviso nos arquivos alterados.
- [x] 4.2 Prova no app de desenvolvimento no `cert`: Ajustes > Modelos lista `claude` e `codex` instalados com caminho e versão; "Testar" e uma conversa de chat com motor CLI respondem (só se o dono autorizar chamadas com as contas do `cert`); capturas e números em `.tmp/verify/cli-agentes-windows/`.
  - `cert`, app de prova em `7a6dbf30`: Modelos mostra `claude` em `C:\Users\cert-\.local\bin\claude.exe · 2.1.304` e `codex` em `C:\Users\cert-\.codex\bin\codex.exe · codex-cli 0.162.1` (`4.2-models-after.png`). Testar do Claude: CLI executado, resposta "Not logged in · Please run /login" (o perfil `cert-` não tem login do Claude Code). Codex não tem Testar por desenho (ferramentas não desligam). Chat: Codex respondeu "ok" (run `completed`); Claude Code devolveu o mesmo "Not logged in" (run `failed`, `agent_cli_failed`), lidos do SQLite. Console bloqueado: capturas pela depuração remota do WebView2 do próprio app.
- [x] 4.3 Limpar o `cert`: parar processos, remover tarefas agendadas, clone, target e toolchains instaladas para a prova; registrar o espaço livre antes e depois.
  - Removidos tarefas, clone/worktree/targets/caches da rodada, dados do app de prova, `C:\bin` criado por engano, temporários da rodada, 15.011 arquivos que o `pnpm install` acrescentou ao store e a toolchain stable (padrão volta a nenhum; 1.97.1 preexistente mantida). C: 53.822.607.360 bytes livres no início, 12.159.123.456 antes da limpeza, 53.686.374.400 depois. Incidente: o primeiro encerramento por árvore de processos também parou `MoNotificationUx.exe` (reuso de PID); script corrigido.
