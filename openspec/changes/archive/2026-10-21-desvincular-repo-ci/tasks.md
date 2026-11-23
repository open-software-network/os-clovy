# Tasks

## 1. CI

- [x] 1.1 Apagar os workflows de release e deploy da organização (D1). — 8 workflows + `.github/actions/phala-deploy` removidos (`9857baaf`).
- [x] 1.2 Trocar runners e ações da Blacksmith nos workflows que ficam; validar com `actionlint` e disparar o workflow `desktop` no fork (`gh workflow run -R guilhermexp/os-clovy`), registrando que os jobs saem da fila e o resultado. — `actionlint` 1.7.7 limpo; `useblacksmith/*` só existia nos workflows apagados; runs 39551206467 e 39553893176: todos os jobs saíram da fila em `ubuntu-latest`/`macos-15`/`windows-2025`; falhas restantes fora do escopo (ver relatório).

## 2. clovy-api local

- [x] 2.1 RED: testes de config e inicialização que exigem subir sem OS Accounts, com `127.0.0.1` padrão e token local, e que `/verify` e o envio ao OS Platform não existem; verificar que falham. — 4 testes falhando em `f8a6d33c` (config default, config.toml em loopback, `/verify` 200≠404, token local 401).
- [x] 2.2 GREEN: D2; testes de 2.1 passam; `cargo test` do workspace `clovy-api` verde (`pnpm test:clovy-api`). — `c6406035`; 522 testes, 0 falhas; fmt e clippy `--all-features -D warnings` limpos.
- [x] 2.3 Subir o `clovy-api` local do dono com o `.env` dele (sem imprimir valores) e fazer uma chamada real a `/healthz` e uma a `/v1/models` com o token local. — na box, com config de teste: `/healthz` 200 e `/v1/models` 200 (429 modelos) com o token local. No Mac, com o `.env` do dono: subiu em `127.0.0.1:8080`, catálogo de 429 modelos, `/healthz` 200, `/v1/models` 200 com o token local.

## 3. Repositório

- [x] 3.1 Remover `CODEOWNERS`, `os-platform.json`, `.conductor/`, skills e entradas do lock ligadas à organização; ajustar `Makefile` (D3). — `e0c0e0f3`; 5 skills e `skills-lock.json` removidos, alvos `skills-*` fora do `Makefile`.
- [x] 3.2 README, SECURITY, CONTRIBUTING, templates de issue e `AGENTS.md` sem links da organização. — `82c5c069`. Ajuste do orquestrador em `docs/`: runbooks de release/runner/atestação apagados ou reescritos, PRD/config/compartilhamento corrigidos, ADRs 0003/0024/0035 superadas (0012/0055 em parte), `CONTEXT.md` (`e0ed3250`..`adb46428`).
- [x] 3.3 Remover `git fetch upstream` de `scripts/qualify-windows-build.ps1`; o orquestrador remove o remote `upstream` do checkout principal ao integrar (D4). — script feito em `ba4598d3`; remote `upstream` e suas refs removidos do checkout principal na integração (só `origin` = guilhermexp/os-clovy).
- [x] 3.4 Guarda D5 no `make verify`. — `41244b09`: `scripts/check-org-references.mjs` (0 ocorrências; 167 na base) + suíte Vitest.

## 4. Verificação

- [x] 4.1 `make verify` verde no Mac. — `make -k verify` saiu 0: Vitest 203 arquivos / 2040 testes, Rust 2735 passaram / 0 falhas / 21 ignorados, guarda de referências limpa.
- [x] 4.2 Evidência em `.tmp/verify/desvincular-repo-ci/` (link do run do CI no fork, saída do `clovy-api` local, varredura da guarda). — `report.md`, `clovy-api-local-calls.txt`, `clovy-api-local.log`.
