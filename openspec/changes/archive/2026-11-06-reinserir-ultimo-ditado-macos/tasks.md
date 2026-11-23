# Tasks

> Executor aprovado pelo usuário: Codex, configuração atual, na agents-us. Seguir `/implement` e TDD. Orquestrador mantém o aceite nativo no Mac; não há autorização para release ou substituição do app real.

**Goal:** permitir Control+V no Mac para reinserir o último ditado sem depender do clipboard.

**Architecture:** Rust retém um último resultado autorizado e qualificado por perfil; o helper reconhece o atalho e insere Unicode no foco atual usando a autoridade e o IPC existentes.

**Tech Stack:** Swift/AppKit/Carbon/CoreGraphics e Rust/Tauri; sem dependências novas.

**Spec:** `specs/dictation/spec.md` e `design.md` desta change.

## Global Constraints

- Base de implementação: QA `72380fc9bd24c172724bc05b1c42376d57ef441f`, em checkout isolado. Não absorver alterações de outras changes.
- Manter Command+V e entrega automática/700 ms, cancelamento, isolamento de perfil e atalhos de captura.
- Não criar persistência, UI/configuração nova, endpoint MCP, digitação frontend irrestrita, fallback de clipboard ou suporte Windows/Linux.
- Não substituir o app real, ler credenciais/dados pessoais, alterar TCC ou repetir provas da paridade.
- Rodar comandos diretamente, sem heavy-run; Cargo jobs=2/sccache inalterados.

## Review Focus

- Resultado cancelado/tardio não substitui o último ditado válido.
- Perfil diferente nunca recebe texto de outro perfil.
- Emoji/pares UTF-16, acentos, texto longo e quebras de linha preservados.
- Tecla segurada, modificadores e resposta IPC tardia não duplicam texto nem mudam destino.
- Conflito/captura de atalho e Acessibilidade ausente não quebram gravação nem Command+V.

## 1. Recuperação explícita do último ditado

Arquivos: `src-tauri/src/dictation.rs` (estado, entrega e evento interno); `src-tauri/native/mac-dictation-helper/main.swift` (Carbon, correlação e Unicode); testes focados seguindo os pontos de teste existentes; `docs/dictation.md`; `AGENTS.md`. Uma extração mínima para testar lógica Swift é permitida somente se necessária, sem novo framework nem abstração de uso único.

- [x] 1.1 Registrar RED antes da implementação: testes comportamentais para substituição de resultado válido, falha de colagem, silêncio/cancelamento/rejeição, perfil diferente e estado inicial vazio; executar `cargo test --lib dictation::tests::last_dictation` em `src-tauri` com identidade de QA. O log deve selecionar casos reais e falhar pelo comportamento ausente, não por erro de ambiente.
- [x] 1.2 Implementar a memória e solicitação correlacionada de reinserção no fluxo autorizado existente. Verificar GREEN dos casos anteriores e dos testes afetados de cancelamento; nenhuma nova transcrição, chamada de limpeza, histórico ou ação de agente ao reinserir. Registrar hashes e comandos no relatório da change.
- [x] 1.3 Adicionar Control+V ao monitor Carbon e inserção Unicode direta; verificar testes focados de seleção macOS, conflito/captura, um disparo por pressionamento, resposta obsoleta, ausência de Acessibilidade e divisão Unicode sem corromper pares UTF-16. Compilar o helper real pelos comandos/flags do `src-tauri/build.rs`, sem simular sucesso do sistema operacional. Atualizar `docs/dictation.md` e `AGENTS.md` com o atalho e limites.

Provas do parent no Mac: 9 regressões novas Tauri + 207 testes restantes do módulo, todos aprovados; Clippy `--lib --tests -D warnings` aprovado. Helper final `e7362840...` compilado integralmente para arm64/x86_64, lipo e 28 checks de lógica/Unicode aprovados. R1: dois interleavings falham no baseline e quatro casos passam no corrigido; revisão independente fechou o achado. Ver `.tmp/verify/reinserir-ultimo-ditado-macos/` (`native/tauri-focused-green.json`, `review-fix/native/parent-verification.json`, logs em `allocation-followup/`). Os checks de estado não comprovam eventos Carbon, TCC real ou inserção consumida: isso continua na tarefa 2.1. Não houve teste visual; o canal nativo de automação falhou ao iniciar, inclusive após alternativa e um reset.

RED inicial do worker, anterior à retenção: seis falhas em sete testes no harness portátil sobre preparação de semântica anterior. Complemento nativo do parent: admissão desativada cirurgicamente em uma linha do módulo real, sete falhas em nove testes, exit101; linha restaurada, SHA integral `967eb7a7e094377f62cb5766c8dbfc6c26c3a466dac27ee1836486837098cb6e` idêntico, mesmos nove testes aprovados, exit0. O complemento é controle negativo, não execução de checkout original intocado. Recibo `.tmp/verify/reinserir-ultimo-ditado-macos/native-memory-red/status.json`; nenhuma mutação temporária permanece.

## 2. Prova integrada no Mac

- [x] 2.1 Compilar e assinar um app de QA próprio com a mudança; registrar fonte/árvore/comandos/saídas. Com o Mac desbloqueado, ditar texto sintético, esperar a restauração do clipboard, focar outro campo e usar Control+V: texto exato uma vez, clipboard anterior intacto. Repetir em campo nativo e web com acento/emoji/nova linha; conferir Command+V, duas reinserções deliberadas, novo ditado válido, cancelamento posterior e nenhum ditado após reinício. Registrar evidência observada e limites sem declarar aceitação por simples emissão de eventos.

  Aceite do dono em 2026-11-06: app de QA `dev.clovy.dr13469713` recompilado do commit `c42b7f34` (build `.tmp/verify/reinserir-ultimo-ditado-macos/dev-run/build.log`, exit 0, assinatura deep/strict ok) e testado manualmente pelo usuário, que relatou "A parte do Ctrl V está funcionando". Limite: o relato não detalha cada caso da matriz (web/nativo, emoji, reinício); nenhuma prova gráfica automatizada.
- [x] 2.2 Revisar somente a entrega desta change e executar o scanner de segredos e gate `/Users/guilhermevarela/orchestrator.v5/scripts/review-pair.sh --repo <checkout> --base 72380fc9bd24c172724bc05b1c42376d57ef441f --out <checkout>/.tmp/verify/reinserir-ultimo-ditado-macos/review-pair/1 --include-untracked --gate`. Indisponibilidade já comprovada exige revisão manual integral registrada, nunca repetição cega ou alegação de aprovação automática. Corrigir achados; entregar recibo, evidências funcionais e commit de escopo, sem integrar/push da paridade por consequência.

Recibo de revisão do checkpoint de código: `.tmp/verify/reinserir-ultimo-ditado-macos/manual-review/1/summary.md`, com scanner integral em `checkpoint-full-scan/1/`. Modalidade manual de contingência; não equivale a aprovação automática nem encerra o aceite gráfico da tarefa 2.1. Build e assinatura do bundle isolado `dev.clovy.dr13469713` aprovados; nenhum app real foi substituído ou executado para esta prova.

## Workflow follow-up

- Após aceite, arquivar esta change pelo CLI OpenSpec e sincronizar somente os AGENTS.md aplicáveis; não arquivar a change de paridade.
- Integrar apenas ao destino explicitamente autorizado para esta entrega. Nenhuma release/deploy nem substituição do app real autorizada aqui.
- Fechar os recursos exclusivos desta change conforme `/cleanup`, preservando o app/target de QA e provas da paridade pausada, trabalho não integrado e dados. Informar remoções, retenções e bytes sem somar recibos antigos novamente.

Fechamento 2.2 (2026-11-06): código do Control+V já comitado em `c42b7f34` com revisão manual de contingência `.tmp/verify/reinserir-ultimo-ditado-macos/manual-review/1/summary.md` e scanner integral; aceite manual do dono registrado em 2.1. O delta seguinte (fixação de destino), que preserva o prefixo Control+V byte a byte, passou no gate automático (`fixar-destino-ditado-no-atalho/review-pair/1`).
