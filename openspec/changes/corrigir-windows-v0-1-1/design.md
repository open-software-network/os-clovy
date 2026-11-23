# Design

## Context

Ver proposal.md. Investigação no código da `v0.1.0` (`dacb1d1e`):

- **Consoles:** nenhum ponto de criação de processo usa `CREATE_NO_WINDOW`. O app de release é `windows_subsystem = "windows"`, então cada filho de console (`cmd.exe` dos shims `.cmd`, `node`, `gh.exe`, `codex.exe`, `claude.exe`, sidecar SEA) abre a própria janela. Na abertura rodam `gh auth status`/`gh auth token` (`connectors/github_cli.rs:67,229`) e o `--version` de cada CLI instalado (`llm/detect.rs:66`), pedido por várias telas ao mesmo tempo (`AgentWorkspace.tsx:430`, `ChatEnginePicker.tsx:131`, `NoteChatPanel.tsx:278`, `LlmProvidersSection.tsx:104`).
- **Codex em notas/atividade:** recusado de propósito em todo sistema (`llm/cli.rs:97`, `llm/registry.rs:470`, `llm/mod.rs:254`). A mensagem de CLI ausente diz "on this Mac" também no Windows (`llm/mod.rs:121`).
- **Codex no chat no Windows:** causa não confirmada no código. Candidatos: sandbox do Codex no Windows (`chat_engine/policy.rs` sempre passa `sandbox_mode`), shim `codex.cmd` num app sem console, e `process_group.rs:35` sem efeito fora do Unix (cancelar/timeout deixam `node`/`codex.exe` vivos).
- **Atalhos:** Fn, tecla sozinha e modificadora sozinha são recusados no Windows por desenho (`dictation.rs:1392–1436`, ADR-0020), e a tela quase não avisa (`AppSettings.tsx:866`). A captura tem duas fontes concorrentes (DOM e gancho do helper) e o cancelamento apaga o erro (`AppSettings.tsx:739,866,933`). AltGr chega como tecla comum e é recusado (`use-shortcut-capture.ts:50`, `dictation.rs:8028`). A falha "não salva nenhum atalho" ainda não foi reproduzida.
- **Janela:** o tratamento de `CloseRequested` que esconde a janela e a função que recria a janela (`lib.rs:781–800, 944–970`) só existem no macOS. No Windows o X destrói a janela; bandeja (`menu_bar.rs:494`), instância única (`lib.rs:142`) e os demais pontos fazem `get_webview_window("main")` e nada acontece quando ela não existe. A bandeja não trata clique no ícone (`menu_bar.rs:144`).

## Goals / Non-Goals

**Goals:** os três relatos resolvidos no Windows de release, provados no Windows de prova; Codex e Antigravity disponíveis em notas, texto e atividade.

**Non-Goals:**
- Fn no Windows (o sistema não recebe a tecla na maioria dos teclados).
- Servidor MCP do Clovy no Windows (o chat por CLI segue sem as ferramentas do Clovy e a interface já indica isso).
- Mudar o comportamento no macOS além do caminho único de mostrar janela.

## Decisions

**D1. Reproduzir antes de corrigir o Codex no chat e o salvamento de atalhos.** O worker reproduz no Windows de prova (`cert`) com o build de release e um `codex` instalado pelo npm, lê o erro real (stderr do Codex, mensagem do atalho, `dictation-settings.json`) e só então corrige a causa encontrada. Hipóteses do design não viram correção sem evidência.

**D2. Um ajuste único sem console.** Uma função `hide_console` aplica `creation_flags(CREATE_NO_WINDOW)` sob `cfg(windows)`, para `std::process::Command` e `tokio::process::Command`. Todo ponto de criação de processo passa por ela, exceto programas gráficos (`explorer.exe`). Um teste falha se surgir `Command::new` fora dela. Sidecar: mesmo ajuste na criação.

**D3. Sondagem compartilhada.** `detect_all` usa um resultado em voo compartilhado mais um cache curto (alguns segundos), para várias telas não dispararem as mesmas sondagens.

**D4. Árvore de processos no Windows.** O processo do CLI é colocado num Job Object com `KILL_ON_JOB_CLOSE`; cancelar ou estourar o tempo fecha o job. Alternativa descartada: `taskkill /T /F`, que é outro processo, mais lento e sujeito a corrida.

**D5. Geração pontual com Codex e Antigravity.** `codex exec` com sandbox `read-only`, aprovação `never`, rede desligada, `--skip-git-repo-check`, `--ephemeral` (ou equivalente que não grava sessão), pasta temporária vazia como diretório de trabalho e prompt pela entrada padrão. `agy`: o worker levanta as flags do CLI instalado e usa a mais restrita (sem escrita e sem aprovação interativa); se o `agy` não tiver modo restrito, roda sem ele na pasta temporária vazia e o design registra isso. A UI deixa de acinzentar os dois nesses usos.

**D6. Atalho de tecla sozinha no Windows.** Modificadora sozinha (RControl, RMenu, RShift) é detectada pelo gancho de baixo nível existente: dispara na soltura quando nenhuma outra tecla foi apertada desde o aperto (empurrar-e-segurar dispara no aperto e solta na soltura, igual ao Fn no Mac). Tecla comum sozinha (F13–F24, Pause, Scroll Lock, Insert…) usa `RegisterHotKey` sem modificadores. Captura no Windows tem uma fonte só: o gancho do helper (a DOM não vê o lado da modificadora nem AltGr de forma confiável). A captura termina sem tecla → mensagem de "tecla não chega ao Windows". Erros ficam na linha até a próxima ação. Padrão do ditado no Windows: Ctrl direito; quem já tem atalho salvo mantém o seu. ADR-0020 ganha adendo datado.

*Atualização v0.1.2 (2026-10-23):* Ctrl, Alt e Shift de qualquer lado valem sozinhos. O gesto é o do Fn no helper do Mac: segurar sozinho por 160 ms abre o microfone até soltar; dois toques em até 340 ms alternam na soltura limpa do segundo toque (no Windows, modificadora sozinha no "alternar" é salva com `pressCount: 2`). Empurrar-e-segurar deixa de começar no aperto, para Ctrl+C, Ctrl+clique e Ctrl+roda não abrirem o microfone; um gancho de mouse de baixo nível vê clique e roda. Entradas injetadas não contam; a repetição vem do próprio gancho e uma soltura perdida não trava o próximo aperto. Detalhes no segundo adendo da ADR-0020.

**D7. Caminho único de mostrar a janela.** Uma função `show_main_window` para todos os sistemas: se a janela existe, mostra, restaura e foca; se não, recria pela configuração (reaproveitando o código do macOS) e registra o ciclo de vida. Bandeja, "Abrir Clovy", instância única, notificações, HUDs e menus passam a chamá-la. `CloseRequested` esconde em todos os sistemas; a parte só do macOS (prewarm do driver, ponte de primeiro clique) continua sob `cfg`. A bandeja trata clique esquerdo no ícone abrindo a janela e mantém o menu no clique direito.

## Risks / Trade-offs

- [Codex lê arquivos da máquina em notas/atividade] → sandbox só leitura e sem rede impedem escrita e envio; aceito pelo dono.
- [Sandbox do Codex não configurado no Windows] → se a causa do chat for essa, o app mostra mensagem clara com o comando de configuração do Codex, em vez de falha genérica.
- [Modificadora sozinha conflita com uso normal] → só dispara sem outra tecla no meio; Ctrl direito+C continua copiando.
- [Gancho de baixo nível com latência] → o helper já usa o gancho para soltura do empurrar-e-segurar; sem custo novo.

## Migration Plan

Merge na `main`, `pnpm version:bump 0.1.1`, tag `v0.1.1`. O app `0.1.0` instalado no Windows do dono oferece a atualização: é também a primeira prova real do updater.
