# Tasks

## 1. Reprodução no Windows de prova (D1)

- [x] 1.1 Instalar a `v0.1.0` publicada no `cert` (perfil `cert-`), com `codex` instalado pelo npm além do nativo; registrar com capturas: janelas de console na abertura, erro real do Codex no chat (stderr ou mensagem da execução), erro real ao salvar um atalho personalizado (mensagem e `dictation-settings.json`), janela que não volta pela bandeja e pela segunda abertura. Registro em `.tmp/verify/corrigir-windows-v0-1-1/reproducao.md`.

## 2. Processos sem console e sondagem compartilhada (D2, D3)

- [x] 2.1 RED: teste que falha com `Command::new` fora de `hide_console`, e teste de sondagem concorrente contando uma sondagem por CLI; commit RED.
- [x] 2.2 GREEN: `hide_console` em todos os pontos de criação de processo (incluindo sidecar) e `detect_all` compartilhado; testes de 2.1 passam.

## 3. Árvore de processos no Windows (D4)

- [x] 3.1 RED: teste Windows com CLI falso `.cmd` que inicia filho longo; cancelar e tempo limite deixam o filho vivo; commit RED.
- [x] 3.2 GREEN: Job Object com encerramento ao fechar, usado pelo chat e pela geração pontual; teste de 3.1 passa no job Windows do CI.

## 4. Codex no chat no Windows (D1)

- [x] 4.1 RED com a falha de sandbox do Codex no Windows emitida por um CLI falso (turno que falha, stderr e comando bloqueado), com os textos do código do Codex (`core/src/exec.rs`: `windows sandbox: {err}`; `windows-sandbox-rs/src/setup_error.rs`: códigos `orchestrator_*`/`helper_*`; TUI: `Windows sandbox setup failed: ...`; remédio do `codex doctor`: `codex sandbox setup --elevated`); commit RED (`64bf3cde`).
- [x] 4.2 GREEN: nesses casos o chat mostra "Codex could not use its Windows sandbox", o comando `codex sandbox setup --elevated --current-user` num terminal de administrador (grava `[windows] sandbox = "elevated"`) e o texto original do Codex; as demais falhas seguem com o texto do Codex.

> Causa no Windows do dono não reproduzida (2026-10-23): no `cert` (`[windows] sandbox = "elevated"`) o chat respondeu com o Codex nativo e com o do npm. O dono mandou seguir sem o erro; o grupo fecha com a mensagem clara para a falha de sandbox, que é a causa mais provável segundo as issues openai/codex#26737 e #37940, e não com uma correção de causa confirmada.

> Em aberto (2026-10-23): no `cert`, a v0.1.0 respondeu no chat com o Codex nativo e com o `codex.cmd` do npm (`.tmp/verify/corrigir-windows-v0-1-1/reproducao.md`, 2b). Não há causa reproduzida para um RED, e D1 proíbe corrigir por hipótese. O que se reproduziu no chat (consoles e processos órfãos ao cancelar) foi corrigido nos grupos 2 e 3. Falta o erro real do Windows do dono (mensagem na tela e o `[windows] sandbox` do `~/.codex/config.toml` dele).

## 5. Codex e Antigravity em notas, texto e atividade (D5)

- [x] 5.1 RED: testes da invocação pontual do `codex` e do `agy` (flags, pasta temporária, sem sessão) e da seleção por uso aceita; `cursor-agent` continua recusado; commit RED.
- [x] 5.2 GREEN: invocações, regras de uso e UI de provedores sem acinzentar os dois; mensagem "neste Mac" corrigida para o Windows; docs de provedores atualizadas.

## 6. Atalhos no Windows (D6)

- [x] 6.1 RED: testes da validação Rust (tecla sozinha e modificadora sozinha aceitas no Windows, Fn recusado), da máquina de estados do gancho (Ctrl direito sozinho dispara; Ctrl direito+C não), da captura de fonte única, da mensagem de tecla não vista e do erro que não some; commit RED.
- [x] 6.2 GREEN: `hotkeys.rs`, `dictation.rs`, `use-shortcut-capture.ts`, `AppSettings.tsx`, padrão Ctrl direito no Windows para quem não tem atalho salvo, e a causa de "não salva nenhum atalho" encontrada em 1.1; testes passam.
- [x] 6.3 Interface da linha de atalho revisada com `/impeccable` (pt-BR e inglês); adendo datado na ADR-0020.

## 7. Janela principal (D7)

- [x] 7.1 RED: testes do caminho único de mostrar janela (recria quando não existe) e do fechar que esconde em todos os sistemas; commit RED.
- [x] 7.2 GREEN: `show_main_window` único usado por bandeja (clique esquerdo e "Abrir Clovy"), instância única, notificações, HUDs e menus; `CloseRequested` esconde em todos os sistemas; testes passam.

## 8. Prova

- [x] 8.1 `make verify` com exit 0 numa worktree limpa; saída em `.tmp/verify/corrigir-windows-v0-1-1/`.
- [x] 8.2 Ensaio do `release.yml` com `dry_run=true` no HEAD final; os quatro jobs verdes.
- [ ] 8.3 Windows de prova com o instalador do ensaio: nenhuma janela de console na abertura nem ao usar chat e notas; Codex responde no chat e gera resumo de nota; Antigravity aceito em atividade (se instalado no `cert`; senão registrar); atalho Ctrl+Shift+D e Ctrl direito salvos e iniciando o ditado; Fn mostra a mensagem; fechar e reabrir pela bandeja e pelo menu Iniciar; cancelar chat não deixa `node`/`codex.exe`. Capturas e registro em `.tmp/verify/corrigir-windows-v0-1-1/windows.md`; desinstalar e limpar no fim.

> 8.3 feita em parte (2026-10-23, `.tmp/verify/corrigir-windows-v0-1-1/windows.md`): provados sem console na abertura, no chat e na chamada pontual; Codex do npm respondendo no chat; cancelar sem `node`/`codex.exe` órfãos; Codex liberado e testado para notas e atividade; Ctrl direito e Ctrl+Shift+D salvos e registrados; mensagem de tecla que não chega ao Windows; fechar esconde e a segunda abertura traz a janela. Sem prova, por a sessão do `cert` estar bloqueada ou faltar dependência: ditado iniciado pelo teclado físico, clique no ícone da bandeja e "Abrir Clovy", resumo de nota com Codex (precisa de áudio e da Clovy API local), Antigravity (não instalado no `cert`).
- [ ] 8.4 macOS: smoke do caminho de janela (fechar e reabrir pela barra de menus e pelo Dock) e de uma geração de nota com `codex`, em app de prova com identificador próprio, sem Keychain.

> 8.4 não feita: um app de prova com identificador próprio cria na abertura a chave do `activity.sqlite3` no Keychain (`activity/key.rs`), e esta execução não pode criar itens no Keychain. Cobertura no macOS: testes de `main_window` e `menu_bar`, e `make verify`.

## Workflow follow-up

- Revisão de código, merge, `pnpm version:bump 0.1.1`, tag `v0.1.1`, release; conferir a atualização automática da `0.1.0` no Windows do dono.
- Arquivar, `/sync`, `/cleanup`.

## 9. Revisão 1 (W1–W5)

- [x] 9.1 W5 RED: `last_message.txt` maior que 8 MiB é lido inteiro; commit RED.
- [x] 9.2 W5 GREEN: o arquivo é lido com o limite de `MAX_CAPTURE_BYTES` e a saída maior é recusada.
- [x] 9.3 W2 RED: o `codex exec` pontual sobe os MCPs e os plugins do usuário; o `agy --print` também; commit RED.
- [x] 9.4 W2 GREEN: o Codex pontual desliga plugins, apps, hooks, `notify` e cada servidor MCP resolvido pelo próprio `codex mcp list --json`, mantendo a config (sandbox do Windows) e a autenticação; o `agy` pontual é recusado com mensagem clara enquanto houver MCP ou plugin ativo (o `agy` 1.3 não tem como desligá-los por chamada); prova com o codex real no Mac (`revisao-1/w2-green-real.txt`: nenhum MCP, plugin ou `notify` sob a chamada).
- [x] 9.5 W3 RED: o prompt inteiro do `agy` no argv passa do limite de linha de comando do Windows sem aviso; commit RED.
- [x] 9.6 W3 GREEN: tamanho da linha de comando do Windows contado em UTF-16 com o escape; acima do limite, erro claro antes de criar o processo, na geração pontual e no chat. O `agy` 1.3 não lê o prompt pela entrada padrão no modo texto ("empty prompt", testado no Mac), por isso ele segue no argv.
- [x] 9.7 W4 RED: Ctrl+F13, Shift+Insert e Ctrl+Pause são recusados na validação e na normalização do Windows; commit RED.
- [x] 9.8 W4 GREEN: essas combinações valem no Windows, na validação e na normalização, sem mudar o macOS; teste do helper confirma o registro com os modificadores (roda no job Windows).
- [x] 9.9 W1 RED: um filho criado entre o `spawn()` e a adoção escapa do Job Object; commit RED.
- [x] 9.10 W1 GREEN: o processo nasce suspenso (`CREATE_SUSPENDED` junto com `CREATE_NO_WINDOW`), entra no job e só então a thread inicial é retomada; falha de criação do job, de atribuição ou de retomada encerra o processo antes de rodar e vira erro visível (chat: "Could not start ...", geração pontual: `llm_cli_failed`, ferramenta de shell: erro da ferramenta). `PROC_THREAD_ATTRIBUTE_JOB_LIST` descartado: `tokio::process::Command` não expõe a lista de atributos.
- [x] 9.11 Grupo 4 RED: falha de sandbox do Codex no Windows aparece como erro genérico no chat; commit RED.
- [x] 9.12 Grupo 4 GREEN: o chat mostra o que configurar (ver 4.2).
- [x] 9.13 `make verify` com exit 0 numa worktree limpa; CI Windows sem falhas novas; ensaio do `release.yml` com `dry_run=true`, os quatro jobs verdes.

> 9.13 (2026-10-23): `make verify` exit 0 em `839a5579`. CI [39958583307](https://github.com/guilhermexp/os-clovy/actions/runs/39958583307) (RED do W1: "child 5388 escaped the job") e [39962757377](https://github.com/guilhermexp/os-clovy/actions/runs/39962757377) (GREEN: Windows `llm` 64, `chat_engine` 26, `process_group` 1, helper 46 ok; falhas só nos vermelhos conhecidos: `activity::windows::clipboard`, vitest `mcp-server-dev-runner`, clippy do macOS com Rust 1.99). Ensaio [39962807756](https://github.com/guilhermexp/os-clovy/actions/runs/39962807756) em `839a5579`, tag `v0.1.0`, `dry_run=true`: os quatro jobs verdes na tentativa 2; a tentativa 1 falhou só no `bundle_dmg.sh` do macOS, depois de compilar, e o job foi repetido sem mudança.
- [x] 9.14 `cert` com o instalador do ensaio: chat com Codex e cancelamento sem órfãos; Codex pontual sem MCP subir; limpeza no fim.

> 9.14 feita (2026-10-23, `.tmp/verify/corrigir-windows-v0-1-1/revisao-1/report.md`): instalador do run 39962807756 (sha256 `3081979a…2101`) em perfil novo, Codex do npm 0.162.1. Chat respondeu "revisado"; cancelar encerrou os 10 processos da árvore (`cmd` → `node` → `codex.exe` → MCPs); o "Testar" do Codex (`run_cli`) deu "strict" em 5,2 s com só `mcp list` e `codex exec` sob o Clovy, nenhum MCP ou plugin; nenhuma janela de console de filhos do Clovy. Desinstalado e limpo. Sem prova no `cert`: W1 só com o ponto de teste (provado no CI Windows), W3/agy (não instalado), W4 com teclado físico (tela bloqueada), grupo 4 com o erro real (sandbox já configurado).

## 10. Retorno do dono na v0.1.1 (v0.1.2)

O dono testou a v0.1.1: no Windows só grava Ctrl sozinho e ele não dispara o ditado; Alt e Fn não gravam; os textos de atividade saem em inglês com a interface em português; o indicador do ditado no Windows mostra ícone genérico. Revisão 2 deixou X1 (agy recusado com plugin desativado) e X2 (dica de sandbox casando erros comuns).

- [x] 10.1 Diagnóstico do gatilho: RED de que cada decisão do gancho (tecla vista, injetada ou não, "sozinha" ou não, tempo segurado, disparo ou descarte e motivo) vira uma linha no registro de eventos do ditado, sem conteúdo digitado; GREEN; commit de cada.
- [x] 10.2 RED: captura e validação aceitam Ctrl, Alt e Shift de qualquer lado sozinhos (Rust, helper, interface); gesto igual ao Fn do Mac (segurar = grava enquanto segura; alternância = o gesto de alternância do Fn no Mac); Ctrl+tecla, Ctrl+clique e Ctrl+roda não iniciam gravação nem mostram indicador; entradas injetadas não contam; tecla sem soltura vista não trava o próximo disparo; commit RED.
- [x] 10.3 GREEN de 10.2: gancho de teclado e de mouse de baixo nível no helper, máquina de estados com espera curta antes de iniciar a gravação de modificadora sozinha (sem abrir o microfone em Ctrl+C), mensagens de captura atualizadas; adendo datado na ADR-0020; testes de 10.2 passam no job Windows.
- [x] 10.4 RED: o indicador do ditado no Windows recebe `target` (nome e identificador do app da janela fixada) no início e na colagem; commit RED.
- [x] 10.5 GREEN de 10.4: o Rust completa o destino pela janela e processo enviados pelo helper, reaproveitando a identificação de app e o ícone que a atividade do Windows já usa; o indicador mostra nome e ícone; teste passa.
- [x] 10.6 RED: com o idioma da interface guardado em português, a geração de atividade na abertura usa português antes de a janela carregar; sem idioma conhecido, nada é gerado; relatórios e resumo do dia corrente em outro idioma entram para nova geração; commit RED.
- [x] 10.7 GREEN de 10.6: o Rust guarda o idioma da interface num arquivo de configuração ao receber o evento e o lê ao abrir; os laços de atividade e de agentes de código esperam o idioma conhecido; nova geração do dia corrente quando o idioma gravado difere; testes passam. Conferir também se a interface do dono pode estar em `en` no armazenamento e a tela parecer em português; registrar.
- [x] 10.8 X1: RED e GREEN — o `agy` só é recusado com plugin ou MCP de fato ativo (plugin desativado não conta), lendo o estado de ativação do próprio `agy`.
- [x] 10.9 X2: RED e GREEN — a dica de sandbox do Codex só aparece para erros de sandbox reconhecidos do Codex no Windows (prefixos e códigos completos), nunca em erro comum de comando nem fora do Windows.
- [ ] 10.10 `make verify` com exit 0 numa worktree limpa; CI Windows (`desktop.yml`) sem falhas novas; ensaio do `release.yml` com `dry_run=true`, quatro jobs verdes.
- [ ] 10.11 `cert` com o instalador do ensaio: captura de Alt esquerdo e Ctrl esquerdo gravada (pelo diagnóstico, já que a tela fica bloqueada: eventos injetados aparecem como injetados e não disparam); indicador com nome e ícone do app de destino; registro em `.tmp/verify/corrigir-windows-v0-1-1/v0-1-2.md`; desinstalar e limpar.

## 11. Revisão da v0.1.2 (Y1–Y5)

A revisão de código da v0.1.2 bloqueou com cinco achados. Cada um tem um commit RED antes do GREEN.

- [x] 11.1 Y1: a alternância por dois toques só dispara na soltura limpa do segundo toque; tecla, clique ou roda entre o segundo aperto e a soltura invalidam o gesto (Ctrl, Ctrl+C não liga o microfone).
- [x] 11.2 Y2: a máscara do Alt também cobre as solturas de Alt consumidas pelo gesto de alternância (a barra de menu do app não abre); acordes reais (Alt+Tab) seguem sem máscara.
- [x] 11.3 Y3: o `finalizing_transcript` do Windows leva o destino fixado na parada, completado no Rust, para o indicador fixar o app que recebe a colagem.
- [x] 11.4 Y4: um resumo do dia apagado pela troca de idioma fica pendente e é refeito assim que os relatórios forem refeitos, uma vez, sem esperar o horário automático.
- [x] 11.5 Y5: `config.json` do `agy` ilegível ou malformado bloqueia a chamada com diagnóstico; só arquivo ausente vale como sem configuração.
- [x] 11.7 Ícone do Windows: as primeiras extrações de ícone de um processo, em paralelo, falham todas (provado em processo filho no CI); a extração pelo Shell passa a ser serializada, e `windows_executables_have_distinct_bounded_png_icons` fica verde no job Windows.
- [x] 11.6 `make verify` numa worktree limpa; CI Windows sem falhas novas; ensaio do `release.yml` com `dry_run=true`; push e evidências.

## 12. Revisão 2 da v0.1.2 (Z1–Z2)

A revisão confirmou Y1, Y3, Y5 e a trava do ícone e deixou dois P2 para a v0.1.3. Cada um tem um commit RED antes do GREEN.

- [x] 12.1 Z1: a pendência `resummary:<dia>:<gatilho>` de ontem também é processada junto do catch-up, e não fica presa depois da meia-noite.
- [x] 12.2 Z2: a máscara da soltura do Alt é decidida de forma síncrona dentro do gancho, sem depender de a mensagem postada já ter sido consumida.
- [x] 12.3 `make verify` numa worktree limpa; CI Windows sem falhas novas; push.
- [x] 12.4 Z2, revisão: uma fonte só para "sozinha". O gancho calcula na hora do evento se a tecla desceu sozinha (`KeySnapshot`), usa isso na máscara e manda na mensagem postada (`KEY_NOT_ALONE_BIT`); o `Gesture` consome essa decisão e não consulta mais o `GetAsyncKeyState`. RED com a ordem C seguro, Alt desce, C solta antes da mensagem, Alt solta.
