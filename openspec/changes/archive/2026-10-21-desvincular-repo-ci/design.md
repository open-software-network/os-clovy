# Design

## Context

O `clovy-api` já roda localmente sem OS Accounts nem OS Platform quando `CLOVY__LOCAL_DEV__ENABLED=true` (`clovy-api/crates/app/src/main.rs:574-626`): token local (`LocalDevTokenVerifier`), cobrança desligada (`LocalDevOsAccountsClient`), P3A e relatórios só em log. Sem essa variável, a validação de config exige `os_accounts.api_url` e chaves reais (`crates/config/src/lib.rs:1307-1375`), e `config.toml` usa `host = "0.0.0.0"`, que com o token padrão é recusado.

## Goals / Non-Goals

**Goals:** nenhum workflow, runner, skill, documento de contribuição ou configuração ligada à organização; CI que de fato roda no fork; `clovy-api` que sobe só em modo local com o token local.

**Non-Goals:** o app desktop (changes `desvincular-servicos-open-software` e `identidade-clovy`); release próprio; mudar os provedores do `clovy-api` (Groq, OpenRouter, fal, Tavily) ou suas chaves.

## Decisions

- **D1 — Workflows.** Apagar os de release e deploy listados na proposta. Nos que ficam, trocar rótulos `blacksmith-*` por `ubuntu-latest`/`macos-15`/`windows-2025`, `useblacksmith/setup-docker-builder` e `useblacksmith/build-push-action` pelas ações `docker/*` oficiais fixadas por SHA, e remover etapas que exigem segredos da organização. O job `windows-rust` continua por disparo manual.
- **D2 — `clovy-api` local.** O modo local vira o único: remover o verificador de tokens OS Accounts, o cliente de cobrança, o envio ao OS Platform, o envio de P3A, a página `/verify` e a config de atestação, `deploy/`, a camada de env `JUNE__` e `scripts/clovy-api-env-compat.mjs`. `server.host` padrão `127.0.0.1`. O token local e o usuário local continuam configuráveis (`CLOVY__LOCAL_DEV__*` ou nomes novos equivalentes; se renomear, manter o antigo como alias por uma versão não é necessário: o único usuário é o dono, e o `.env.example` e o `.env` documentado são atualizados juntos).
- **D3 — Skills e ferramentas.** Remover skills e entradas do lock que dependem da organização; manter as genéricas (`browser-test-tauri-fe`, `agent-e2e-qa` se não depender do OS Platform após ajuste, `speckit-*`). Remover a seção do OS Platform no `AGENTS.md`.
- **D4 — Remote.** `git remote remove upstream` no checkout do dono; nada no repositório volta a referenciá-lo.
- **D5 — Guarda.** Estender o script de guarda do `make verify` (criado na change de serviços, ou criado aqui se esta entrar antes) para cobrir `.github/`, `clovy-api/`, `scripts/` e `Makefile` com `opensoftware\.co\b|june\.link|open-software-network|blacksmith`.

## Risks / Trade-offs

- Runners do GitHub são mais lentos e o macOS conta minutos com peso maior → aceitável para uso privado; o gate local `make verify` continua sendo a referência.
- Remover código do `clovy-api` pode quebrar testes de integração que fixam OS Accounts → apagar os testes dos caminhos removidos, manter os de modo local.
- Skills do dono no harness externo (`~/orchestrator.v5`) não são tocadas.

## Migration Plan

O `.env` local do `clovy-api` do dono precisa continuar subindo: o worker confere, sem imprimir segredos, que as chaves que ele usa ainda existem com o mesmo nome ou atualiza o `.env.example` e informa as renomeações.
