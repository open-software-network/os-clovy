# Proposal

## Why

Este fork é de uso privado do dono, no Mac e no Windows, com modelos locais, CLIs e chaves próprias, e um `clovy-api` próprio rodando localmente. O app ainda depende de serviços da Open Software (empresa do produto June, de onde veio o fork):

- Um build de release sem `OS_CLOVY_LOCAL_DEV` para no primeiro passo do onboarding pedindo login na conta Open Software e nunca abre o app (`src/lib/account-gate.ts:10`, `OnboardingFlow.tsx`, `SignInStep.tsx:165-170`, `AccountGate.tsx:110-112`).
- O padrão do servidor é `https://june-api.opensoftware.co` e o dos links de compartilhamento é `https://june.link` (`src-tauri/src/clovy_api.rs:29-30`).
- Cobrança, indicações e checagem de plano Pro/Max, que hoje também bloqueia o Computer Use (`computer_use.rs:2011`).
- Liberação remota do Computer Use, que falha fechada sem o servidor (`computer_use.rs:1758-1783`).
- Telemetria P3A, envio de relatórios de erro e o app companheiro (app iOS da Open Software).
- O atualizador aponta para `open-software-network/os-june-releases` e confia na chave deles (`tauri.conf.json:116-121`, `updates.rs:22,26`).
- Links de termos e privacidade em `accounts.opensoftware.co` e da comunidade no Telegram.

## What Changes

- **BREAKING** O app passa a ter só identidade local: abre sem login, sem passo de conta no onboarding e sem portão de conta. O OS Accounts sai por inteiro (comandos `os_accounts_*`, tokens no Keychain, esquema `osjune://`, OAuth scopes).
- **BREAKING** Saem cobrança, indicações, avisos de financiamento, checagem de plano, telemetria P3A, envio de relatórios de erro e o app companheiro.
- O Computer Use deixa de depender de liberação remota e de plano: fica disponível conforme as condições locais que já existem (plataforma, permissões, assinatura do driver).
- As chamadas ao `clovy-api` usam o token local; o endereço padrão passa a ser `http://127.0.0.1:8080`, com `CLOVY_API_URL` sobrepondo; o padrão `june.link` sai e os links de compartilhamento usam a origem do servidor configurado.
- O atualizador da June sai (endereços, chave e verificação de atualização); o atualizador próprio fica para a change de release.
- Saem links para domínios da Open Software (termos, privacidade, Telegram) e código morto ligado a eles (`fetch_browser_transport_policy`).

## Capabilities

### New Capabilities
- `local-account`: o app funciona só com identidade local, sem conta, cobrança, telemetria ou atualizador de terceiros.

### Modified Capabilities

## Impact

- Rust: `os_accounts.rs`, `clovy_api.rs`, `commands.rs`, `computer_use.rs`, `audio/capture.rs`, `meeting_detection.rs`, `dictation.rs`, `p3a/`, `companion/`, `updates.rs`, `lib.rs` (registro de comandos e plugins), `tauri.conf.json` (updater, esquema `osjune`), `Cargo.toml` se dependências ficarem sem uso.
- Frontend: `AccountGate`, `SignInStep`, `OnboardingFlow`, `TelemetryConsentStep`, `account-gate.ts`, `app-account-gates.tsx`, `AccountSettings`, `FundingNotice`, `billing-actions.ts`, `ReferralNudge`, `max-upgrade`, `Sidebar`, `AppSettings`, chamadas `p3aRecord`, tela de relatório de erro, i18n.
- Testes e docs que fixam esses fluxos (`docs/os-accounts-login.md`, `docs/companion-*`, `docs/llm-providers.md`).
- Dados: tokens antigos do OS Accounts no Keychain ficam órfãos (não são apagados).
- Fora do escopo: workflows, repositório e `clovy-api` (change `desvincular-repo-ci`); nome e identificador (change `identidade-clovy`); release próprio.
