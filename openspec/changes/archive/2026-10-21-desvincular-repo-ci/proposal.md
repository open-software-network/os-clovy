# Proposal

## Why

O repositório do fork `guilhermexp/os-clovy` ainda carrega a infraestrutura da Open Software (empresa do produto June, de onde veio o fork):

- workflows de release e deploy amarrados à organização: GitHub App `owner: open-software-network`, releases em `open-software-network/os-june-releases`, tap `open-software-network/homebrew-tap`, imagem `ghcr.io/open-software-network/clovy-api`, deploys Phala/Cloudflare de `june-api.opensoftware.co` e `june.link`, watchdog, Slack;
- todos os jobs em runners da Blacksmith (`blacksmith-*`, `useblacksmith/*`) ou num Mac Studio da organização, que nunca são atendidos no fork: os jobs ficam na fila para sempre;
- remote `upstream`, `CODEOWNERS` com `@junhohong`, README, SECURITY, CONTRIBUTING e templates de issue com links da organização;
- skills de projeto e `skills-lock.json` que puxam repositórios privados da organização (`os-accounts`, `skills`) e falam com o OS Platform, mais `os-platform.json`;
- o `clovy-api` ainda tem configuração e código de produção da organização: verificação de tokens e cobrança pelo OS Accounts, envio de relatórios ao OS Platform, página de atestação Phala (`/verify`), compose de deploy e compatibilidade `JUNE__`.

## What Changes

- Remover os workflows que só servem à organização: release (`rc-desktop-dmg`, `staging-desktop-dmg`, `promote-desktop`, `production-desktop-windows`), API (`build-clovy-api`, `promote-clovy-api`, `clovy-api-watchdog`) e `deploy-clovy-link`. O release próprio volta numa change separada.
- Trocar os runners da Blacksmith dos workflows que ficam (`desktop`, `coverage`, `clovy-api`, `cargo-release-age`, `repository-hygiene`) por runners do GitHub (`ubuntu-latest`, `macos-15`, `windows-2025`) e as ações `useblacksmith/*` pelas oficiais.
- `clovy-api` só em modo local: sem verificador OS Accounts, sem cobrança, sem envio ao OS Platform, sem P3A, sem `/verify` de atestação, sem `deploy/` e sem a camada `JUNE__`; servidor em `127.0.0.1` por padrão.
- Remover o remote `upstream` e o `git fetch upstream` de `scripts/qualify-windows-build.ps1`.
- Remover `CODEOWNERS`, `os-platform.json`, `.conductor/` e as skills de projeto que dependem de serviços ou repositórios privados da organização (certamente `os-platform`, `os-accounts-integration` e `os-task-prep`; as demais `os-*` e `repo-*` são conferidas uma a uma: sai a que chama OS Platform ou repositório da organização, fica a que só descreve o código), com as entradas do `skills-lock.json`.
- README, SECURITY, CONTRIBUTING e templates de issue sem links da organização; `AGENTS.md` sem a seção do OS Platform.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

- `.github/workflows/*`, `.github/CODEOWNERS`, `.github/ISSUE_TEMPLATE/*`, `README.md`, `SECURITY.md`, `CONTRIBUTING.md`, `AGENTS.md`, `skills-lock.json`, `Makefile`, `os-platform.json`, `.conductor/`, `.agents/skills/*` e espelhos em `.claude/skills/*`, `scripts/*`.
- `clovy-api/`: `config.toml`, `crates/config`, `crates/app/src/main.rs`, `crates/api` (`/verify`, autenticação, cobrança), `crates/providers/src/issue_reports.rs`, `deploy/`, `.env.example`, testes que fixam esses valores.
- `.git/config` local (remote).
- Sem mudança de requisito de produto: o app desktop não muda nesta change.
