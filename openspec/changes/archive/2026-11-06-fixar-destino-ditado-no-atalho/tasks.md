# Tasks

> Executor: Codex na agents-us (mesmo da change `reinserir-ultimo-ditado-macos`), seguindo `/implement` e TDD. O orquestrador compila o helper e o app de QA no Mac e conduz a prova manual com o dono.

**Goal:** o destino da colagem automática do ditado é o app, a janela e o campo em uso no acionamento do atalho, até o fim da tomada.

**Base:** branch `feat/clovy-dictation-repeat-20261105`, commit `c42b7f34fe69798ff73356f37da726f2ef815e1b` (inclui o Control+V aceito pelo dono).

## Global Constraints

- Não mudar o Control+V, prazos de ativação/colagem, restauração do clipboard, cancelamento, perfis ou atalhos.
- Sem dependência, permissão ou configuração nova; sem Windows/Linux.
- Não substituir o app real, não ler dados pessoais, não alterar TCC.

## 1. Fixação no acionamento

- [x] 1.1 RED: testes da lógica real de `FocusTargetController` com adaptadores sintéticos: A no acionamento e B ativado durante a fala mantêm destino e metadados em A; stop não repina; cancelamento/entrega limpam; nova tomada captura de novo; fora da tomada o candidato acompanha o foco. Devem falhar no código atual pelo comportamento.
- [x] 1.2 Capturar o destino (app + janela + campo AX, com timeout curto) no acionamento e mantê-lo pela tomada; `focus_target`/`listening_started`/`finalizing_transcript` e `targetBundleIdentifier` usam o fixado; remover a repinagem do stop. GREEN de 1.1 e testes afetados (Rust/HUD, se tocados).
- [x] 1.3 Na entrega, trazer a janela original e restaurar o campo; colar só com app e janela da tomada na frente; destino inválido → `paste_target_unavailable` sem Command+V. Testes com janelas sintéticas do mesmo app e janela inválida.
- [x] 1.4 Atualizar `docs/dictation.md`, ADR 0014 e `AGENTS.md`.

## 2. Prova e revisão

- [x] 2.1 Compilar o helper real (flags do `build.rs`) e o app de QA `dev.clovy.dr13469713`; o dono repete o cenário relatado (outra janela abre durante a fala) e o de duas janelas do mesmo app. Registrar resultado observado.
- [x] 2.2 Revisão do delta desde `c42b7f34` pelo gate `/Users/guilhermevarela/orchestrator.v5/scripts/review-pair.sh --gate` ou revisão manual de contingência registrada, com scanner de segredos; commit de escopo na branch.

## Evidências (2026-11-06)

- Worker Codex (agents-us), relatório `.tmp/verify/fixar-destino-ditado-no-atalho/implementation/report.md`, patch SHA256 `3f25a9ff2ce46710c19220e34abbe671b696f6ffe9deebb168f929db534a14b0`. Captura em `DictationController.start`, antes da permissão de microfone; AX app-scoped com timeout ≤20 ms.
- Mac: `run-regressions.sh` RED exit 1 (7 passaram/17 falharam no baseline `c42b7f34`), GREEN exit 0 (24/24). Helper integral arm64+x86_64 + lipo, sem erros; `--self-test-last-dictation` 28 passaram.
- App de QA integrado `dev.clovy.dr13469713` (árvore `qa-integracao-20261106`, build exit 0, assinatura deep/strict ok); o dono testou e respondeu "parece que ta tudo ok". Limite: relato geral, sem detalhamento caso a caso.
- Gate `review-pair.sh --gate` sobre o delta desde `c42b7f34` (com untracked): exit 0, OpenQodex e CodeRabbit concluídos, 0 achados, gitleaks limpo. Recibo `.tmp/verify/fixar-destino-ditado-no-atalho/review-pair/1/summary.md`.
