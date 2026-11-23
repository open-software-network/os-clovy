# Tasks

## 1. Identidade local

- [x] 1.1 RED: testes do módulo de identidade local (token padrão e sobreposto, usuário local, sempre conectado) e do onboarding/portões do frontend chegando à tela principal sem conta; verificar que falham.
  - Evidência: `f5706b71`; RED em `.tmp/verify/desvincular-servicos-open-software/logs/red-1.1-rust.log` (5 falhas) e `red-1.1-frontend.log` (2 falhas).
- [x] 1.2 GREEN: D1 no Rust (remover OS Accounts, esquema `osjune` e escopos; chamadores usam a identidade local) e D2 no frontend; testes de 1.1 passam.
  - Evidência: `3dc4bf52`; módulo `local_identity.rs`; `1.2-focused.log` (0 falhas) e `1.2-test.log` (só a falha conhecida `mcp-server-dev-runner`).

## 2. Remoções

- [x] 2.1 Cobrança, indicações, financiamento, upgrade e checagens de plano (D2), apagando os testes desses fluxos.
  - Evidência: `038439fe` (antes do GREEN, para o 1.2 compilar sem os comandos de cobrança); `2.1-test.log`.
- [x] 2.2 Computer Use sem conta (D3): só o necessário para não depender do OS Accounts; a remoção da liberação remota e da checagem de plano já está na change `runtimes-cli-na-ui` (branch `feat/runtimes-cli-na-ui`, tarefas 3.1/3.2) e não é refeita aqui.
  - Evidência: entrou em `3dc4bf52` (`plan_eligible` passa a ser verdadeiro sem conta; teste `computer_use_needs_no_account_plan`). A remoção do OS Accounts obrigou a mudança no mesmo commit; rollout e checagem de plano ficaram para `runtimes-cli-na-ui`. Em `3c405abd` saiu também a permissão remota do companheiro no `computer_use.rs`.
- [x] 2.3 P3A, relatórios de erro, companheiro, `fetch_browser_transport_policy` e links da Open Software (D4), com dependências sem uso.
  - Evidência: `3c405abd`; `2.3-focused.log`, `2.3-test.log`.
- [x] 2.4 Atualizador da June (D6): config, plugin, `updates.rs` e UI.
  - Evidência: `f7f068d6`; `2.4-test.log`.

## 3. Servidor e guarda

- [x] 3.1 RED/GREEN de D5: padrão `http://127.0.0.1:8080`, `CLOVY_API_URL` sobrepõe, links de compartilhamento pela origem do servidor.
  - Evidência: RED `cecbb459` (`red-3.1-rust.log`, 2 falhas), GREEN `46190041` (`green-3.1-rust.log`).
- [x] 3.2 Guarda D7 no `make verify`, falhando antes das remoções e passando depois.
  - Evidência: `edd243f7`; `guard-d7-base.log` (26 ocorrências na base) e `guard-d7-after.log` (limpo).

## 4. Documentação

- [x] 4.1 Atualizar ou remover docs afetadas (`docs/os-accounts-login.md`, `docs/companion-*`, `docs/llm-providers.md`, README do app) e conferir com o código.
  - Evidência: `9954f10f` (11 docs apagados, docs/CONTEXT/AGENTS atualizados, ADRs 0001, 0003, 0012, 0041–0048, 0051–0053 marcadas como superadas); testes `.mjs` e `4.1-test.log` sem falha nova. Código morto do companheiro e do P3A no repositório SQLite saiu em `647e46cf`.

## 5. Verificação

- [x] 5.1 `make verify` verde no Mac. — `make -k verify` saiu 0 em `8ded3b92`: Vitest 185 arquivos / 1783 testes, Rust 2631 passaram / 0 falhas / 21 ignorados (as duas rodadas anteriores pegaram código morto só-macOS e o teste de integração `storage.rs`, corrigidos em `002caef7` e `8ded3b92`).
- [x] 5.2 Prova no app de desenvolvimento com identificador próprio da execução e perfil novo vazio: onboarding sem passo de conta até a tela principal; Ajustes sem Cobrança; uma nota transcrita e um ditado pelo `clovy-api` local; Computer Use disponível sem plano; capturas e números em `.tmp/verify/desvincular-servicos-open-software/`. — App `co.opensoftware.june.unlinkproof` com pasta de dados nova e armazenamento web vazio: onboarding de 3 etapas (foco, personalidade, permissões) sem passo de conta, termina no Início sem login; Ajustes com Geral, Aparência, Atalhos, Atividade, Ditado, Áudio, Modelos, Agente, Memória, Plugins e Sobre, sem Cobrança; capturas em `run-mac/` no checkout principal. Não provados no app: nota transcrita e ditado (o perfil de prova não tem as chaves de provedor do dono) e Computer Use sem plano (a remoção do portão está na `runtimes-cli-na-ui`). A inicialização automática que o app de prova gravou (`June.plist`) foi desligada pelos Ajustes e o arquivo sumiu.
