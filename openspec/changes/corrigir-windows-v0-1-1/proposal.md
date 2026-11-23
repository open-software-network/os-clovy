# Proposal

## Why

O dono instalou a v0.1.0 no Windows e relatou três problemas: ao abrir o app aparecem 3–4 janelas de terminal e o Codex não funciona como runtime (nem em notas, texto e atividade, nem no chat); não dá para gravar um atalho diferente do padrão, e o Fn não funciona; e, depois de fechar a janela, o app continua na bandeja mas a janela não volta.

## What Changes

- Nenhum processo filho abre janela de console no Windows (sondagem de CLIs, `gh`, chat, chamadas pontuais, sidecar, MCP, ferramentas). A sondagem de CLIs é compartilhada em vez de repetida por cada tela.
- Codex e Antigravity (`agy`) passam a valer para notas, limpeza de texto e atividade, rodando no modo de menor privilégio que cada CLI oferece (sem escrita, sem rede quando possível, pasta temporária vazia, sem sessão persistida). Decisão do dono.
- Codex como motor do chat funciona no Windows, inclusive instalado pelo npm (`codex.cmd`); cancelar e tempo limite encerram a árvore de processos inteira no Windows.
- Atalhos no Windows: qualquer atalho válido pode ser salvo, os erros aparecem e não somem; uma tecla sozinha passa a valer (Ctrl direito, Alt direito/AltGr, Shift direito, F13–F24, Pause e afins); o ditado no Windows usa Ctrl direito por padrão; quando a tecla apertada não chega ao Windows (caso do Fn), a tela diz isso.
- Fechar a janela principal no Windows a esconde; clicar no ícone da bandeja, em "Abrir Clovy" ou abrir o app de novo traz a janela de volta.
- Mensagens que dizem "neste Mac" no Windows passam a falar do computador.

## Capabilities

### New Capabilities

- `desktop-window`: ciclo de vida da janela principal (fechar, esconder, reabrir pela bandeja e por segunda abertura) no Windows e no macOS.

### Modified Capabilities

- `llm-providers`: Codex e Antigravity liberados para geração pontual em modo de menor privilégio; processos de CLI sem console e encerrados por inteiro no Windows.
- `dictation`: atalho de tecla sozinha no Windows e avisos de captura.

## Impact

- `src-tauri/src/llm/` (cli, detect, process, registry, mod), `chat_engine/` (turn, cli, policy, mod), `connectors/github_cli.rs`, `agent_runtime/` (host, tools, migration), `agent_mcp.rs`, `process_group.rs`, `lib.rs`, `menu_bar.rs` e os outros pontos que mostram a janela principal.
- `src-tauri/native/windows-dictation-helper/src/hotkeys.rs`, `dictation.rs` (validação e lista de teclas).
- `src/components/shortcuts/use-shortcut-capture.ts`, `src/components/settings/AppSettings.tsx`, `LlmProvidersSection.tsx`, mensagens i18n.
- `docs/adr/0020-windows-dictation-keyboard-hook.md` (adendo), docs de provedores.
- Release `v0.1.1` depois do merge, que também prova a atualização automática a partir da `v0.1.0`.
