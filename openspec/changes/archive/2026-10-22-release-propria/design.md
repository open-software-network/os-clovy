# Design

## Context

Ver proposal.md (Why). Estado atual, levantado no código da branch `feat/identidade-clovy`:

- Updater: não existe. O commit `f7f068d6` ("feat: remove the June updater") apagou `src-tauri/src/updates.rs`, `src/lib/updater.ts`, `src/app/update-decision.ts`, `src/app/use-app-update-actions*.ts`, `src/app/app-effects/update-ui.tsx`, a parte de atualização de `shutdown.rs`, `AppSettings.tsx` e `app.css`, os testes `update-decision.test.ts` e `updater.test.ts` e as dependências `tauri-plugin-updater`/`tauri-plugin-process`. O código antigo tinha dois canais (estável e rc) apontando para `open-software-network/os-june-releases`.
- Build do Mac: `scripts/build-signed-dmg.sh` monta o build universal (Node 24 oficial arm64 e x64, sidecar SEA, auxiliares, `tauri build --target universal-apple-darwin`), mas exige identidade Developer ID e notarização. Os scripts de auxiliares (`bundle-nm-shim.sh`, `bundle-clovy-mcp.sh`, `build-agent-runtime.mjs`, `build.rs`) já caem para assinatura ad hoc quando `APPLE_SIGNING_IDENTITY` não existe.
- Build do Windows: `scripts/build-windows.ps1` gera o NSIS sem assinatura e `verify-windows-artifacts.ps1` exige `NotSigned`. O job "Tauri Windows build" do `desktop.yml` roda o script só em disparo manual.
- Versão `0.0.40` em `package.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock` e `src-tauri/tauri.conf.json`; `pnpm version:bump X.Y.Z` atualiza os quatro. Não há tags `vX.Y.Z`.
- `guilhermexp/os-clovy` é público, sem releases e sem secrets.
- Migração de identidade (change `identidade-clovy`): roda no macOS antes do Tauri Builder, de produção para produção e de dev para dev.

## Goals / Non-Goals

**Goals:**
- Uma tag publica Mac e Windows juntos, com atualização automática verificada pela chave do dono.
- Assinatura Apple e notarização ligáveis depois só com secrets, sem mudar código.
- Prova completa antes do merge com um modo de ensaio que não publica.

**Non-Goals:**
- Assinatura Authenticode no Windows.
- Canal rc ou escolha de canal.
- Build só arm64 do Mac (o universal já é a receita existente).
- Publicar a extensão do navegador.
- Fixar o toolchain Rust e os vermelhos já conhecidos do `desktop.yml` (ficam na change "CI verde no fork").

## Decisions

**D1. Releases no próprio `guilhermexp/os-clovy`.** O repositório é público, então `https://github.com/guilhermexp/os-clovy/releases/latest/download/latest.json` responde sem token. Alternativa descartada: repositório separado de releases como o June fazia; só valia porque o código do June era privado.

**D2. Updater restaurado do commit `f7f068d6^`, com um canal só.** Restaurar o Rust, o hook React, a decisão de oferecer atualização, o adiamento do reinício durante gravação/ditado (`shutdown.rs`) e a ação em Ajustes, e cortar o que era do canal rc (`ReleaseChannel`, `release-settings.json`, `latest-rc.json`). O endpoint fica só em `tauri.conf.json` (`plugins.updater.endpoints`), sem endpoint em código. Itens visuais restaurados (aviso e Ajustes → Geral) passam por `/impeccable`. Alternativa descartada: escrever do zero; o código antigo já resolvia reinício seguro e tinha testes.

**D3. Só o build de release verifica.** A verificação automática e a ação em Ajustes ficam desligadas em build dev (`cfg!(debug_assertions)` no Rust; o front lê essa informação do comando). Assim `make dev` nunca baixa um release por cima do bundle dev.

**D4. Artefatos de atualização só no workflow.** `tauri.conf.json` ganha `plugins.updater` com a chave pública e o endpoint, e `bundle.createUpdaterArtifacts` continua desligado. O workflow passa `src-tauri/tauri.release.conf.json` (novo, com `createUpdaterArtifacts: true`) pelo `--config`. Builds locais sem a chave privada continuam funcionando.

**D5. Chave do updater.** Par gerado com `pnpm tauri signer generate`, com senha aleatória. Privada e senha em `~/.config/clovy/updater.key` e `~/.config/clovy/updater.key.password` (modo 0600, mesmo lugar da chave da extensão), e nos secrets `TAURI_SIGNING_PRIVATE_KEY` e `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` gravados com `gh secret set` lendo do arquivo (nunca na linha de comando nem em log). Perder a chave obriga a reinstalar à mão; o dono guarda uma cópia (instrução no doc).

**D6. Mac: um script de release com assinatura opcional.** `build-signed-dmg.sh` vira `scripts/build-macos-release.sh` (corte limpo: o antigo e o script `tauri:build:signed-dmg` saem, entra `tauri:build:release-mac`). Sem `APPLE_SIGNING_IDENTITY`, assina ad hoc e pula notarização; com identidade, assina; com identidade e chave da App Store Connect, notariza e grampeia o `.dmg`. O script exporta `CARGO_PROFILE_RELEASE_BUILD_OVERRIDE_STRIP=none`. Lê variáveis do ambiente (CI) ou do `.env.signing` (local).

**D7. Windows: o script atual mais a assinatura do updater.** `build-windows.ps1` continua sem Authenticode; com `TAURI_SIGNING_PRIVATE_KEY` presente, passa `tauri.release.conf.json` e copia também o `.sig` para a saída. O verificador continua exigindo `NotSigned` no Authenticode e passa a exigir o `.sig` quando a chave existe.

**D8. Workflow `release.yml`.** Gatilhos: push de tag `v*.*.*` e `workflow_dispatch` com `tag` e `dry_run` (padrão `true`). Jobs:
1. `version` (ubuntu): `scripts/check-release-version.mjs <tag>` confere a tag com os quatro arquivos e que o release não existe.
2. `macos` (`macos-15`) e `windows` (`windows-2025`), em paralelo, sobem os pacotes como artefatos do run.
3. `publish` (ubuntu, `needs` os três, `permissions: contents: write` só aqui): `scripts/release-manifest.mjs` gera o `latest.json` a partir dos `.sig`; com `dry_run` só sobe o manifesto como artefato; sem `dry_run`, `gh release create <tag> --generate-notes` com os seis arquivos.
Ações fixadas por SHA como no `desktop.yml`; `concurrency` por tag; permissões padrão `contents: read`.

**D9. Release não migra.** A chamada de `identity_migration` fica sob `cfg(all(target_os = "macos", debug_assertions))` e o mapeamento da origem de produção (`co.opensoftware.june` → `dev.guilhermexp.clovy`) sai do código, sem sobra morta. O guard `check-org-references.mjs` continua passando porque o nome antigo só aparece na origem dev.

**D10. Primeira versão `0.1.0`.** `pnpm version:bump 0.1.0`, commit `release: v0.1.0`, tag `v0.1.0` na `main` depois do merge. A prova de atualização usa `v0.1.1`.

**D11. Limpeza.** `stable-release-provenance.mjs` (nomes `June_*`) e as receitas que passam `createUpdaterArtifacts:false` são ajustados ou apagados conforme o uso real; o texto "the fork has no updater signing key" no `AGENTS.md` e em `docs/release-macos.md` é corrigido. Nova ADR do updater próprio substitui a nota de remoção na ADR-0001 (append-only: adendo datado).

## Risks / Trade-offs

- [Assinatura ad hoc muda a cada build] → o macOS pede de novo microfone, gravação de tela, acessibilidade e áudio a cada atualização. Aceito pelo dono; some quando os secrets Apple entrarem (D6).
- [Download pelo navegador marca quarentena] → o primeiro `.dmg` sem notarização abre com "Abrir assim mesmo" em Ajustes do Sistema → Privacidade e Segurança. Doc explica. A prova confere se a atualização pelo app também cai na quarentena.
- [Perda da chave do updater] → instalações existentes não aceitam pacote de outra chave; reinstalar à mão. Cópia local + instrução de backup.
- [Runner do Mac lento e caro] → o build universal leva tempo; aceito para releases manuais e raros. Sem build de release em PR.
- [Toolchain Rust flutuante] → um stable novo pode quebrar o build de release; tratado na change "CI verde no fork".
- [Release antes da C na `main`] → a tag real só é criada na `main` depois do merge da C e desta change; antes disso, só ensaio (`dry_run`).

## Migration Plan

1. Implementar na branch `feat/release-propria` (base `feat/identidade-clovy`; rebase na `main` depois do merge da C).
2. Ensaio: `gh workflow run release.yml --ref feat/release-propria -f tag=v0.1.0 -f dry_run=true`, conferindo artefatos, assinaturas e manifesto.
3. Merge na `main`, bump `0.1.0`, tag `v0.1.0`: release real.
4. Prova de atualização: bump `0.1.1` e tag; o app `0.1.0` instalado oferece e instala a `0.1.1` (Windows na máquina de prova; Mac na aceitação do dono).
5. Rollback: apagar o release com problema (`gh release delete`) faz o `latest` voltar ao anterior; quem já atualizou reinstala o `.dmg`/instalador anterior.
