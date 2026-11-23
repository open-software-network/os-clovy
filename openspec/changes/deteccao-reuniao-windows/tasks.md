# Tasks

## 1. Plano

- [x] 1.1 Proposal, design, spec delta `meeting-detection` e tasks; `openspec validate deteccao-reuniao-windows --strict`; commit do plano.

## 2. Sonda e lista no Windows (D2, D3, D4, D5)

- [x] 2.1 RED: testes do parser (`windows_input_processes`: sessão WASAPI ativa, subida Chromium e WebView2, exclusão de `owned_pids` depois da subida, reforço do registro com e sem processo, entradas fora da lista ignoradas), da lista e dos rótulos do Windows, e da máquina de estados (aviso e fim) com processos do Windows; commit RED (`87e1b39c`, 14 testes falhando).
- [x] 2.2 GREEN: parser puro em `meeting_detection/windows_probe.rs`, tabela `WINDOWS_MIC_APPS` (no mesmo arquivo, para o harness de prova compilar o arquivo real), rótulo sem `.exe`; testes de 2.1 passam (`9f833ae3`).

## 3. Leitura no Windows e monitor (D1, D6)

- [x] 3.1 Leitura WASAPI, Toolhelp e registro sob `cfg(target_os = "windows")`; `active_input_processes` do Windows usa o parser (`9f833ae3`).
- [x] 3.2 `spawn_monitor`, `observe_meeting_end` e `setup` ligados no macOS e no Windows (`9f833ae3`); job `windows-rust` roda `meeting_detection` no host Windows.
- [x] 3.3 W1 (P2 da revisão): a contagem do fim no HUD aparece sempre que o Clovy não está em primeiro plano no Windows. `app_inactive()` compara o pid de `GetForegroundWindow` (`activity/windows/window.rs::foreground_window_pid`) com o do Clovy pela função pura `clovy_inactive_for_foreground`; o macOS não muda. RED `d2236727`, GREEN `9fe1ddef`.

## 4. Docs

- [x] 4.1 `docs/audio-pipeline.md` descreve a sonda do Windows e o reforço do registro. O `AGENTS.md` não descreve a detecção de reunião e fica igual.

## 5. Verificação

- [x] 5.1 `make verify` numa worktree limpa (`9fe1ddef`, `verify_exit=0`).
- [x] 5.2 CI Windows (`desktop.yml`) verde, salvo os vermelhos conhecidos (run 40580351390: meeting detection 47/47 nas duas tentativas; tentativa 1 com todos os testes verdes e falha só na limpeza do runner, tentativa 2 no flake conhecido de clipboard; clippy macOS 1.99 e `mcp-server-dev-runner.test.ts` conhecidos).
- [x] 5.3 Windows de prova: binário de teste com captura do microfone visto pela sonda, o próprio processo excluído, fim observado; relatório em `.tmp/verify/deteccao-reuniao-windows/report.md`. O W1 (primeiro plano) não tem prova nativa; ver "Sem prova".
