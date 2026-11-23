# Proposal

## Why

No Windows o Clovy não encontra os agentes CLI instalados (`claude`, `codex` e os demais), enquanto no Mac eles aparecem sozinhos. O dono viu isso no app do Windows. A leitura do código mostra a causa: a resolução foi escrita só para Unix.

- O Windows guarda o PATH na variável `Path`; `LoginEnv::path_dirs` e `append_fallback_dirs` procuram só a chave `PATH`, então a lista de diretórios fica vazia.
- `LoginEnv::which` testa o nome sem extensão (`claude`); no Windows os executáveis são `claude.exe`, `codex.cmd` e afins (PATHEXT). Pior: o npm grava ao lado do `.cmd` um script `sh` sem extensão, que passaria em `is_file()` e não executa.
- Os diretórios de reserva são de Unix (`~/.local/bin` via `HOME`, `/opt/homebrew/bin`); no Windows não existe `HOME` e os instaladores usam `%APPDATA%\npm`, `%USERPROFILE%\.local\bin` e outros.
- A captura do shell de login (`$SHELL -l`, padrão `/bin/zsh`) não faz sentido no Windows e só falha.

Todo recurso que roda um CLI depende disso: lista de provedores, "Testar", escolha por uso (atividade, notas, ditado), chat com motor CLI e resumos de sessões de agentes.

## What Changes

- Resolver o ambiente de execução dos CLIs no Windows a partir do ambiente do próprio app, sem shell de login, lendo `Path` sem diferenciar maiúsculas e gravando de volta na mesma chave.
- Procurar cada CLI no Windows com as extensões do `PATHEXT` (padrão `.COM;.EXE;.BAT;.CMD`), nunca o arquivo sem extensão.
- Acrescentar, no Windows, os diretórios onde os instaladores oficiais desses seis CLIs gravam quando o PATH não os traz (`%APPDATA%\npm`, `%USERPROFILE%\.local\bin` e os confirmados por instalador).
- Garantir que um CLI resolvido como `.cmd`/`.bat` rode com o prompt íntegro (texto com várias linhas, aspas, `%`, `^`, `&`), inclusive nos CLIs que recebem o prompt como argumento (`agy`, `cursor-agent`, `copilot`).
- macOS sem mudança de comportamento.

## Capabilities

### New Capabilities

### Modified Capabilities
- `llm-providers`: o requisito "Catálogo de provedores CLI detectados" passa a definir a resolução no Windows (ambiente do app, `Path`, `PATHEXT`, diretórios dos instaladores) e a execução íntegra de CLIs `.cmd`.

## Impact

- `src-tauri/src/llm/shell_env.rs` (`LoginEnv::which`, `path_dirs`, `append_fallback_dirs`, `login_env`, `resolve_login_env`).
- Execução: `src-tauri/src/llm/process.rs`, `src-tauri/src/chat_engine/turn.rs` e `chat_engine/cli.rs` (prompt por argumento).
- Consumidores de `which`: `llm/detect.rs`, `llm/commands.rs`, `llm/mod.rs`, `chat_engine/mod.rs`, `coding_agents/summarize.rs`.
- `docs/llm-providers.md`.
- Prova no Windows `cert`, onde `claude` e `codex` já estão instalados.
- Fora do escopo: outros usos de `HOME` sem relação com CLIs (extensão do navegador, raízes de skills do agente, lançador de navegador).
