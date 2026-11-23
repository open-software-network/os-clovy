# Proposal

## Why

O fork não tem release: a change `desvincular-servicos-open-software` removeu o updater e o canal de releases do June, e a `desvincular-repo-ci` removeu os workflows de publicação. Hoje o dono só tem o build dev no Mac e um instalador Windows sem atualização, gerado à mão no CI. A identidade própria (`identidade-clovy`) fecha o caminho para um release do próprio dono, com atualização automática, no Mac e no Windows.

## What Changes

- Workflow de release no GitHub Actions do `guilhermexp/os-clovy`: uma tag `vX.Y.Z` (ou disparo manual com a tag) gera o build universal do Mac (`.dmg` e `.app.tar.gz`) e o instalador NSIS do Windows, assina os pacotes de atualização com a chave do updater do dono e publica um GitHub Release com os pacotes e o `latest.json`.
- A tag só publica se bater com a versão gravada nos quatro arquivos de versão.
- Atualização no app volta, só para builds de release: verifica ao abrir e pelo botão em Ajustes, mostra a versão nova, instala e reinicia quando o dono aceita. Um único canal (estável), apontado para os releases do próprio repositório, verificado pela chave pública do dono. Build dev nunca verifica.
- Chave própria do updater: par minisign gerado para o dono; a privada fica num secret do repositório e numa cópia local fora do repositório; a pública vai na configuração do app.
- Mac sem assinatura Apple por enquanto (assinatura ad hoc, sem notarização). O workflow assina com identidade Apple e notariza só quando os secrets correspondentes existirem, sem outra mudança.
- Windows continua sem assinatura Authenticode.
- **BREAKING** (só para o build de release do Mac): o release abre com perfil vazio; a migração única da identidade June passa a rodar só no build dev do macOS. O perfil de produção `co.opensoftware.june` (parado desde setembro) não é mais lido.
- Limpeza do que sobrou do release do June nos scripts (`June_*` em `stable-release-provenance.mjs`) e nas instruções que desligam artefatos de updater que não existem.

## Capabilities

### New Capabilities

- `app-updates`: como o app de release encontra, mostra, verifica e instala uma versão nova, e quando não verifica.
- `release-distribution`: como uma versão vira release publicado para Mac e Windows (gatilho, consistência de versão, pacotes, manifesto de atualização, assinatura opcional do Mac).

### Modified Capabilities

- `app-identity`: a migração única da identidade June deixa de rodar no build de release; só o build dev do macOS migra. (Capability criada pela change `identidade-clovy`, que é arquivada antes desta.)

## Impact

- `src-tauri/`: dependências `tauri-plugin-updater` e `tauri-plugin-process`, bloco `plugins.updater` em `tauri.conf.json`, capability, registro no `lib.rs`, comandos de verificar e instalar; condição de build no ponto de chamada de `identity_migration`.
- `src/`: aviso de atualização e ação em Ajustes → Geral (com `/impeccable`), mensagens i18n.
- `.github/workflows/release.yml` novo; `desktop.yml` sem mudança de comportamento.
- `scripts/`: build de release do Mac com assinatura opcional, geração do `latest.json`, verificação de versão da tag; ajustes no `build-windows.ps1` para gerar o pacote de atualização assinado.
- Secrets do repositório: `TAURI_SIGNING_PRIVATE_KEY` e `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (obrigatórios); secrets Apple opcionais.
- Docs: `docs/release-macos.md`, `docs/release-windows.md`, ADR nova do updater próprio, `AGENTS.md`.
- Depende da change `identidade-clovy` mergeada na `main`.
