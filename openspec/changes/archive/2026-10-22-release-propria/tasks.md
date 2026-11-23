# Tasks

## 1. Release não migra (D9)

- [x] 1.1 RED: teste `the_release_build_never_migrates` falha com a chamada atual; commit RED.
- [x] 1.2 GREEN: chamada de `identity_migration` sob `cfg(all(target_os = "macos", debug_assertions))` e origem de produção removida sem código morto; `cargo test identity_migration` e `node scripts/check-org-references.mjs` passam.

## 2. Chave e configuração do updater (D4, D5)

- [x] 2.1 Gerar o par com `pnpm tauri signer generate` e senha aleatória; privada e senha em `~/.config/clovy/updater.key{,.password}` com modo 0600; conferir com `stat` e que nada entrou no repositório (`git status`).
- [x] 2.2 Gravar `TAURI_SIGNING_PRIVATE_KEY` e `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` com `gh secret set -R guilhermexp/os-clovy` lendo do arquivo; conferir com `gh secret list` (só nomes).
- [x] 2.3 `plugins.updater` em `tauri.conf.json` (chave pública e endpoint `releases/latest/download/latest.json`), `src-tauri/tauri.release.conf.json` com `createUpdaterArtifacts: true`, dependências `tauri-plugin-updater` e `tauri-plugin-process` (Cargo e package.json), capability e registro no `lib.rs`; `pnpm tauri:build` local sem a chave privada continua passando (`make verify`). Desvio: só o plugin Rust; sem `tauri-plugin-process` nem pacotes JS, porque o front chama comandos próprios (`fetch_update`, `install_update`, `relaunch_for_update`) e o reinício usa `request_restart`, como no updater de `f7f068d6^`; sem capability de plugin pelo mesmo motivo.

## 3. Atualização no app (D2, D3)

- [x] 3.1 RED: restaurar de `f7f068d6^` os testes `update-decision.test.ts`, `updater.test.ts` e os de `shutdown.rs`, ajustados para canal único e build dev sem verificação (`updates_are_disabled_in_dev_builds`); commit RED.
- [x] 3.2 GREEN: restaurar `updates.rs`, `src/lib/updater.ts`, `update-decision.ts`, `use-app-update-actions*.ts`, `update-ui.tsx`, o adiamento do reinício em `shutdown.rs` e a ação em Ajustes → Geral, sem `ReleaseChannel`, `release-settings.json` nem endpoint rc; testes de 3.1 passam.
- [x] 3.3 Interface do aviso e de Ajustes → Geral revisada com `/impeccable` (pt-BR e inglês nas mensagens i18n); capturas no app de prova em `.tmp/verify/release-propria/`.
- [x] 3.4 ADR nova do updater próprio e adendo datado na ADR-0001; `docs/index.md` atualizado.

## 4. Build do Mac (D6)

- [x] 4.1 RED: teste do script de release do Mac decidindo entre ad hoc, identidade e notarização pelas variáveis presentes (modo de checagem, sem `codesign`); commit RED.
- [x] 4.2 GREEN: `scripts/build-macos-release.sh` (a partir de `build-signed-dmg.sh`, que sai junto com `tauri:build:signed-dmg`), com `CARGO_PROFILE_RELEASE_BUILD_OVERRIDE_STRIP=none` e `tauri.release.conf.json` quando houver chave do updater; teste de 4.1 passa.
- [x] 4.3 Build local ad hoc com a chave do updater: gera `Clovy_universal.dmg`, `Clovy.app.tar.gz` e `.sig`; `verify-macos-app-name.mjs`, `lipo -archs` (arm64 e x86_64) e `codesign --verify --deep --strict` passam; saídas em `.tmp/verify/release-propria/mac-local/`.
- [x] 4.4 `docs/release-macos.md`: receita nova, variáveis opcionais da Apple, quarentena no primeiro download e permissões pedidas de novo a cada atualização com assinatura ad hoc.

## 5. Build do Windows (D7)

- [x] 5.1 RED: teste do verificador exigindo o `.sig` quando a chave do updater existe; commit RED.
- [x] 5.2 GREEN: `build-windows.ps1` passa `tauri.release.conf.json` e copia o `.sig` quando há chave; `verify-windows-artifacts.ps1` exige o `.sig` nesse caso e continua exigindo `NotSigned` no Authenticode; `docs/release-windows.md` atualizado.

## 6. Workflow e manifesto (D8, D10)

- [x] 6.1 RED: `src/test/release-version-check.test.mjs` (tag igual, diferente, release existente) e teste do gerador do `latest.json` (três plataformas, URLs do release, conteúdo dos `.sig`); commit RED.
- [x] 6.2 GREEN: `scripts/check-release-version.mjs` e `scripts/release-manifest.mjs`; testes de 6.1 passam.
- [x] 6.3 `.github/workflows/release.yml` conforme D8 (ações por SHA, permissões mínimas, `concurrency`, `dry_run` padrão `true`); YAML válido e `actionlint` sem erro.
- [x] 6.4 Ensaio: `gh workflow run release.yml -R guilhermexp/os-clovy --ref feat/release-propria -f tag=v0.0.40 -f dry_run=true`; os três jobs passam, os seis arquivos saem como artefatos do run e cada `.sig` confere com a chave pública; link do run e conferência em `.tmp/verify/release-propria/dry-run.md`.

## 7. Limpeza (D11)

- [x] 7.1 `stable-release-provenance.mjs` e seus testes ajustados aos nomes do Clovy ou apagados se nada mais os usa; receitas com `createUpdaterArtifacts:false` e o texto "no updater signing key" corrigidos em `AGENTS.md`, `docs/development.md` e `docs/release-macos.md`; `make verify` com exit 0.

## 8. Prova

- [x] 8.1 `make verify` com exit 0 e saída em `.tmp/verify/release-propria/make-verify.txt`.
- [x] 8.2 Windows na máquina de prova (`ssh cert`, só no perfil `cert-`, sem credencial do GitHub): instalar o `setup.exe` do ensaio, abrir, conferir versão e que nenhuma migração rodou; desinstalar e apagar o perfil criado; registro em `.tmp/verify/release-propria/windows.md`.

## 9. Revisão 1 (N1–N3)

- [x] 9.1 N1: aceitar a atualização com um ditado já ativo espera o ditado; cada passo lê `dictationWorkflowActiveRef.current` de forma síncrona e o polling só detecta o fim (`src/test/use-app-update-actions.test.tsx`).
- [x] 9.2 N2: `install_update` baixa, espera a trava de atualização (`src-tauri/src/update_gate.rs`) e só então instala; gravação ativa, iniciando ou finalizando e ditado ativo ou recém-iniciado seguram a instalação (evento `WaitingForCapture`, cartão de espera, "Mais tarde" cancela); com a instalação em curso, gravação e ditado recusam o início com `update_installing`.
- [x] 9.3 N3: `relaunch_for_update` resolve com o resultado do shutdown; um reinício cancelado pela barreira de notas rejeita com `update_relaunch_cancelled`, o hook limpa `relaunching` e o cartão permite tentar de novo.
- [x] 9.4 `make verify` numa worktree limpa do HEAD, saída em `.tmp/verify/release-propria/make-verify-revisao-1.txt`.
- [x] 9.5 Novo ensaio com `dry_run=true` no HEAD final; conferência em `.tmp/verify/release-propria/dry-run.md`.
- [x] 9.6 N2 mudou a instalação no Windows: prova 8.2 repetida na `ssh cert` com a mesma limpeza; registro em `.tmp/verify/release-propria/windows-revisao-1.md`.

## 10. Revisão 2 (R1–R3)

- [x] 10.1 R1: a trava guarda os takes de ditado em curso por id, sem prazo; um take entra quando o início é admitido ou quando o helper avisa que começou, e um take que o helper começa durante a instalação é descartado.
- [x] 10.2 R2: o take da trava só termina no evento terminal real (`paste_completed`, que também confirma o composer, descarte, `agent_session_prompt` ou erro); `final_transcript` e `paste_target` não terminam; a trava não lê mais a flag da bandeja.
- [x] 10.3 R3: adiamento e admissão decididos sob o mesmo lock; adiamento antes da vaga impede a instalação, depois dela é recusado com `update_install_started`; evento `Installing` tira o cartão de espera e o "Mais tarde".
- [x] 10.4 `make verify` numa worktree limpa, saída em `.tmp/verify/release-propria/make-verify-revisao-2.txt`.
- [x] 10.5 Novo ensaio com `dry_run=true`; conferência em `.tmp/verify/release-propria/dry-run.md`.
- [x] 10.6 `install_update` no Windows mudou (evento `Installing`, adiamento na trava): prova 8.2 repetida na `ssh cert`; registro em `.tmp/verify/release-propria/windows-revisao-2.md`.

## 11. Revisão 3 (S1–S2)

- [x] 11.1 S1: `handle_helper_event_line` aplica o fim do take na trava pelo id do próprio evento antes dos filtros de take atual e de interface; `final_transcript(A) → início(B) → paste_completed(A) → terminal(B)` deixa a trava vazia.
- [x] 11.2 S1, varredura: descarte, erro, `agent_session_prompt`, recuperação por cópia (erro) e perda do helper passam pelo mesmo ponto; um take que o Rust cancela ou substitui sai da trava ao cancelar o token (a transcrição dele volta sem evento), e um `recording_ready` stale sai em `claim_finalized`; um take já entregando continua até o `paste_completed`.
- [x] 11.3 S2: o helper do Mac recusa com `error` `recording_start_busy` e o id do take um toggle que chega durante a finalização; o helper do Windows já recusa o início durante `awaiting_transcript` com o mesmo erro e o id, sem mudança.
- [x] 11.4 `make verify` numa worktree limpa, saída em `.tmp/verify/release-propria/make-verify-revisao-3.txt`.
- [x] 11.5 Novo ensaio com `dry_run=true`; conferência em `.tmp/verify/release-propria/dry-run.md`. A prova 8.2 não se repete: o caminho de instalação do Windows (`install_update`) não mudou.

## Workflow follow-up

- Depois do merge da `identidade-clovy` na `main`: rebase desta branch, revisão, aceite e merge.
- Na `main`: `pnpm version:bump 0.1.0`, commit `release: v0.1.0`, tag `v0.1.0` → release real; conferir `releases/latest/download/latest.json` sem autenticação.
- Prova de atualização: bump `0.1.1` e tag; no Windows de prova o `0.1.0` instalado oferece e instala o `0.1.1`; no Mac, o dono instala o `.dmg` `0.1.0` (Abrir assim mesmo) e aceita a atualização.
- Arquivar (`openspec archive release-propria -y`, depois da `identidade-clovy`), `/sync`, `/cleanup`.
