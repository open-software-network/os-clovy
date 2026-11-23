# Design

## Context

`LoginEnv` (`src-tauri/src/llm/shell_env.rs`) é a única fonte de ambiente e de resolução de executáveis para os CLIs. Hoje ele captura `$SHELL -l` (padrão `/bin/zsh`), procura a chave `PATH`, acrescenta `~/.local/bin`, `~/.npm-global/bin`, `~/.bun/bin`, `~/.volta/bin`, `/opt/homebrew/bin` e `/usr/local/bin`, e `which` junta o nome puro do CLI a cada diretório, aceitando qualquer arquivo no Windows (`is_executable` = `is_file`). Os processos rodam com `env_clear()` + `envs(...)` (`llm/process.rs`, `chat_engine/turn.rs`). `agy`, `cursor-agent` e `copilot` recebem o prompt como argumento (`chat_engine/cli.rs`); os demais pela entrada padrão.

## Goals / Non-Goals

**Goals:** os seis CLIs aparecem no Windows como no Mac quando estão instalados; rodam com o mesmo ambiente do usuário; o prompt chega íntegro; macOS sem mudança.

**Non-Goals:** outros usos de `HOME` sem relação com CLIs; reler o `Path` do registro durante a execução do app (o Mac também captura o ambiente uma vez por execução); instalar CLIs pelo Clovy.

## Decisions

- **D1 — Sem shell de login no Windows.** `login_env` usa o ambiente do processo (herdado do Explorer, que já traz o `Path` de usuário e de máquina) e não tenta `$SHELL -l`. Alternativa rejeitada: ler `HKCU\Environment` e `HKLM\...\Session Manager\Environment`, que só ajuda quando o CLI foi instalado com o app aberto, caso que o Mac também não cobre.
- **D2 — Chave do PATH sem diferenciar maiúsculas.** No Windows, leitura e escrita usam a chave existente que case com `PATH` ignorando maiúsculas (normalmente `Path`), para não criar duas chaves que o processo filho resolveria de forma ambígua.
- **D3 — `PATHEXT`.** No Windows, `which(nome)` testa, em cada diretório, `nome` + cada extensão de `PATHEXT` (padrão `.COM;.EXE;.BAT;.CMD` quando ausente), na ordem do `PATHEXT`, e nunca o nome sem extensão. No Unix nada muda.
- **D4 — Diretórios de reserva por plataforma.** Windows: `%APPDATA%\npm`, `%USERPROFILE%\.local\bin`, `%USERPROFILE%\.bun\bin`, `%LOCALAPPDATA%\Volta\bin`, mais os diretórios que os instaladores oficiais de `claude`, `codex`, `pi`, `agy`, `cursor-agent` e `copilot` comprovadamente usam (confirmar cada um pela documentação oficial ou pelo instalador e registrar a fonte). Unix: lista atual.
- **D5 — Prompt em `.cmd`/`.bat`.** O Rust (≥ 1.77) escapa argumentos para arquivos em lote e devolve erro quando não consegue escapar com segurança. Primeiro provar no Windows, com um `.cmd` falso, o que chega para prompts com várias linhas, aspas, `%`, `^` e `&`. Se algum caso alterar o texto ou falhar, a correção vai na origem: para shims do npm, executar `node` com o script JS que o shim chama; ou enviar o prompt pela entrada padrão quando o CLI aceitar. Escolher com evidência e registrar.

## Risks / Trade-offs

- Prompt por argumento em `.cmd` pode ter limite de tamanho da linha de comando (8191 caracteres no `cmd.exe`) → o teste do D5 inclui um prompt longo; acima do limite, falha explícita ou entrada padrão/`node` direto.
- Diretórios de instalação mudam entre versões dos CLIs → cada entrada tem fonte registrada; o `Path` do usuário continua sendo a fonte principal.
- O `cert` é máquina de outra pessoa; os CLIs instalados lá usam a conta dela → só `--version` e chamadas de teste autorizadas pelo dono.

## Migration Plan

Sem migração de dados. As escolhas salvas de provedor por uso continuam válidas; no Windows um provedor antes "não instalado" passa a aparecer instalado.
