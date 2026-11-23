# Design

## Context

`os_accounts.rs` já tem um modo local (`local_dev_enabled`, `os_accounts.rs:466-469`), sem `cfg(debug_assertions)`, que devolve conta assinada, usuário `usr_local_dev`, assinatura ativa e o token `OS_CLOVY_LOCAL_DEV_BEARER_TOKEN` (`local-dev-token` por padrão). Ele é ligado só pela variável de ambiente `OS_CLOVY_LOCAL_DEV`, lida do `.env` em caminhos que dependem da máquina de build, então não serve para um release no Windows. O `clovy-api` em modo local (`CLOVY__LOCAL_DEV__ENABLED=true`) aceita esse token e não fala com OS Accounts nem OS Platform.

## Goals / Non-Goals

**Goals:** app abre e funciona sem conta em qualquer build; nenhum endereço da Open Software no app; sem cobrança, P3A, relatórios de erro, companheiro, liberação remota e atualizador da June; padrão do servidor em `127.0.0.1:8080`.

**Non-Goals:** mudar o `clovy-api` (change `desvincular-repo-ci`); renomear June/identificador (change `identidade-clovy`); atualizador e release próprios; configurar endereço do servidor pela interface (o release próprio decide como o Windows chega ao servidor).

## Decisions

- **D1 — Identidade local fixa.** Remover o OS Accounts e deixar um módulo pequeno de identidade local que fornece id de usuário (`usr_local_dev`), token (`OS_CLOVY_LOCAL_DEV_BEARER_TOKEN` ou `local-dev-token`) e "conectado = sempre". Todos os chamadores de `access_token`, `current_user_id`, `cached_signed_in` e `stored_user_id` passam a usar esse módulo. Alternativa rejeitada: só ligar `local_dev` por padrão, que manteria código morto de login, Keychain e esquema `osjune`.
- **D2 — Frontend sem conta.** Remover `AccountGate`, `SignInStep`, passo de conta do onboarding, portões de financiamento, aba Cobrança, indicações, upgrade Max e checagens de plano. O estado de conta some do frontend; onde a UI mostrava o usuário, usar só o perfil local já existente, se houver, senão remover.
- **D3 — Computer Use local.** A remoção da busca de `/v1/computer-use/rollout` e da checagem de plano já está na change `runtimes-cli-na-ui` (aguardando aceite do dono). Aqui só se remove o que depende do OS Accounts (a leitura de plano via conta passa a considerar a identidade local), mudando o mínimo em `computer_use.rs` para reduzir conflito com aquela branch.
- **D4 — Remoções.** P3A (`p3a/`, comandos, passo de consentimento, `p3aRecord`), relatórios de erro (`/v1/issue-reports`, comando e tela), companheiro (`companion/`, ajuste experimental, comandos), `fetch_browser_transport_policy`, links de termos/privacidade/Telegram. Dependências que ficarem sem uso saem do `Cargo.toml`/`package.json`.
- **D5 — Servidor.** `DEFAULT_CLOVY_API_URL = "http://127.0.0.1:8080"`, `CLOVY_API_URL` em tempo de execução e de build continua sobrepondo; `DEFAULT_SHARE_BASE_URL` sai e links usam a origem do servidor.
- **D6 — Atualizador.** Remover `plugins.updater` do `tauri.conf.json`, `createUpdaterArtifacts`, `updates.rs` e a UI de atualização/canal; o plugin de updater sai do `lib.rs`. A change de release o reintroduz apontando para o repositório do dono.
- **D7 — Guarda.** Script em `scripts/` chamado pelo `make verify` que falha se `src/`, `src-tauri/src/` ou `src-tauri/*.json` casarem com `opensoftware\.co\b|june\.link|open-software-network`.

## Risks / Trade-offs

- Remoção grande em código compartilhado → um commit por área (backend conta, frontend conta, cobrança, P3A, relatórios, companheiro, Computer Use, atualizador), cada um com `make verify` verde.
- Testes que fixam fluxos removidos → apagar junto (não reescrever para fixar o vazio).
- Tokens antigos no Keychain ficam órfãos → aceitável; nada é lido do Keychain para apagá-los.
- A troca de nome e identificador (change seguinte) mexe em `tauri.conf.json` e `lib.rs` → esta change entra primeiro.

## Migration Plan

Sem migração de dados. Na primeira abertura após a mudança, o app entra direto; preferências de onboarding já concluídas continuam valendo.
