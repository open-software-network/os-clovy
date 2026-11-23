## Purpose

Define como uma versão do Clovy vira um release publicado no repositório do dono, com pacotes para Mac e Windows e o manifesto que o app usa para se atualizar.

## ADDED Requirements

### Requirement: Gatilho e consistência de versão
Um release SHALL ser gerado por push de tag `vX.Y.Z` em `guilhermexp/os-clovy` ou por disparo manual informando essa tag. O workflow MUST falhar antes de qualquer build quando a tag não bater com a versão de `package.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock` e `src-tauri/tauri.conf.json`, ou quando um release com essa tag já existir.

#### Scenario: Tag igual à versão
- **WHEN** os quatro arquivos dizem `0.1.0` e a tag `v0.1.0` é enviada
- **THEN** o workflow segue para os builds
- Test: unit — script de verificação da tag (`src/test/release-version-check.test.mjs`)

#### Scenario: Tag diferente da versão
- **WHEN** a tag é `v0.1.1` e os arquivos dizem `0.1.0`
- **THEN** o workflow falha no primeiro job, sem build e sem release
- Test: unit — mesmo script, caso de divergência

### Requirement: Pacotes publicados
Um release completo SHALL conter: o `.dmg` universal do Mac, o `.app.tar.gz` universal com seu `.sig`, o instalador NSIS do Windows (`*-setup.exe`) com seu `.sig` e o `latest.json`. O `latest.json` SHALL listar `darwin-aarch64`, `darwin-x86_64` e `windows-x86_64`, cada um com a URL do pacote neste release e o conteúdo do `.sig` correspondente. O release MUST ser publicado só depois que os dois builds terminarem; se um falhar, nada é publicado.

#### Scenario: Os dois builds passam
- **WHEN** os jobs do Mac e do Windows terminam com sucesso
- **THEN** o release `vX.Y.Z` aparece como mais recente com os seis arquivos, e `releases/latest/download/latest.json` responde sem autenticação
- Test: integration — primeiro release real (prova da change) e teste unitário do gerador do `latest.json`

#### Scenario: Um build falha
- **WHEN** o job do Windows falha
- **THEN** nenhum release aparece para a tag, nem com pacotes parciais; a tag pode ser reenviada depois de corrigir
- Test: estrutura do workflow (job de publicação depende dos dois builds), conferida por `actionlint`

### Requirement: Assinatura dos pacotes
Os pacotes de atualização SHALL ser assinados com a chave privada do updater do dono, lida só dos secrets do repositório; a chave MUST NOT aparecer em log, artefato ou arquivo do repositório. O build do Mac SHALL usar assinatura ad hoc quando não houver secrets de identidade Apple, e SHALL assinar com a identidade e notarizar quando os secrets de identidade e de notarização existirem. O instalador do Windows não recebe assinatura Authenticode.

#### Scenario: Sem secrets Apple
- **WHEN** o repositório só tem os secrets do updater
- **THEN** o build do Mac sai com assinatura ad hoc, sem notarização, e o release é publicado
- Test: integration — primeiro release real

#### Scenario: Com secrets Apple
- **WHEN** os secrets de identidade e de notarização da Apple existem
- **THEN** o app, os auxiliares e o `.dmg` saem assinados por essa identidade e o `.dmg` é notarizado e grampeado
- Test: unit — teste do script de build do Mac decide entre ad hoc e identidade pelas variáveis presentes (sem executar `codesign`)
