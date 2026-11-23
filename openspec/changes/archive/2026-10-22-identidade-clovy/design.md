# Design

## Context

`app_paths.rs` deriva as pastas do identificador do Tauri (`-dev` em debug). `build.rs` exporta `CLOVY_APP_IDENTIFIER` da config, mas cerca de 12 pontos de produção fixam o identificador (`extension_host.rs:98`, `computer_use_driver.rs:484,556`, `activity/filter.rs:11`, `build.rs:450,715`, `cua-driver-pin.json`, `main.swift:1165`, scripts). Nenhum código move a pasta de dados; o único precedente de cópia é `migrateDevWebKitStore` em `scripts/dev-app-bundle.mjs:196`. Credenciais têm nomes fixos; `credential_compat.rs` faz leitura dupla canônico/legado para a maioria, mas não para `activity/key.rs` (chave SQLCipher) nem `llm/secrets.rs`. A ADR-0055 declara o identificador June imutável para proteger usuários publicados; aqui o único usuário é o dono.

## Goals / Non-Goals

**Goals:** nenhum vestígio de June/Open Software na identidade; o dono abre o app novo com todos os dados e credenciais; nada antigo é apagado.

**Non-Goals:** release próprio e assinatura de distribuição; renomear o produto para outro nome além de Clovy.

## Decisions

- **D1 — Nomes.** Identificador `dev.guilhermexp.clovy`; auxiliares `dev.guilhermexp.clovy.computer-use-driver`, `.audio-capture`, `.dictation-helper`; executável `clovy`; `Clovy.app`, `Clovy Computer Use Driver.app`, `Clovy Dictation Helper.app`, `clovy-system-audio-recorder`, `clovy-dictation-helper`, `clovy-nm-shim`; inicialização automática "Clovy"; editor e `authors` "guilhermexp". Uma constante única no Rust derivada do `CLOVY_APP_IDENTIFIER` de `build.rs` substitui os literais.
- **D2 — Credenciais.** Prefixo de serviço `dev.guilhermexp.clovy` (dev `dev.guilhermexp.clovy-dev`). A migração lê cada item pelos nomes antigos (`co.opensoftware.clovy*`, `co.opensoftware.june*`), grava no novo e passa a ler só o novo; `credential_compat.rs` deixa de existir depois disso. A chave SQLCipher é copiada antes de abrir `activity.sqlite3`; se não puder ser copiada, a migração inteira falha (D3) para não criar banco novo com chave nova.
- **D3 — Migração de dados.** Módulo `identity_migration` chamado no início do `setup`, antes de qualquer banco: se a pasta nova não existe ou está vazia e a antiga existe, copia para uma pasta temporária irmã, confere contagem e tamanhos, renomeia para o nome final (troca atômica no mesmo volume) e só então migra credenciais. Falha → remove a temporária, registra, segue com pasta nova vazia. Cobre `app_data_dir` e `app_config_dir`, produção e dev, e o armazenamento do WebView (`~/Library/WebKit/<id>`). **Só macOS** (quarta revisão, decisão do dono): no Windows não há perfil June, o app sempre começa limpo e o módulo nem compila lá. Uma falha grava o rollback no journal (com fsync) antes de apagar; a abertura seguinte termina esse rollback ou bloqueia. Execuções de agente pausadas são aposentadas na cópia (cartões expirados, execução e sessão `interrupted`, sem `interrupted_state_json`), porque o estado do SDK guarda caminhos antigos dentro de uma string opaca. A lista de caminhos inclui `hermes/config.yaml` (MCP), `hermes/cron/jobs.json` (`script`, `workdir`) e `obsidian.json` (`vaultPath`).
- **D4 — Extensão.** Nova chave RSA no `manifest.json` (id novo), host `dev.guilhermexp.clovy.extension` no app e na extensão, `validate-extension-id.mjs` e `extension_host.rs` com o id novo; o host antigo `june.extension` sai. A extensão é carregada sem empacotar a partir do build.
- **D5 — Esquema de URL.** Só `clovy://`; `osjune` sai (o login que o usava já saiu na change de serviços).
- **D6 — Compatibilidade June.** Removidas: `OS_JUNE_*` em `env_compat.rs`, `storage-compat.ts` legado, alias `june-nm-shim`, varredura de temporários June em `launcher.rs`, helpers legados de persona/skill. Fica só a lista de origens da migração.
- **D7 — ADR.** Nova ADR substitui a 0055 registrando a troca de identidade e a migração.

## Risks / Trade-offs

- Perda de dados se a migração falhar no meio → cópia em pasta temporária e troca atômica; origem nunca é alterada.
- Permissões do macOS (Acessibilidade, Gravação de Tela, Microfone, Notificações, Arquivos) para o app novo e auxiliares → o dono concede de novo uma vez; a prova registra o que foi pedido.
- Keychain pode pedir autorização para ler itens antigos com outra assinatura → é decisão do dono na tela; a migração de credencial que for negada falha a migração inteira (sem estado misto).
- Configuração MCP colada em clientes aponta para a pasta antiga → a tela de MCP mostra o caminho novo; o dono recola.
- Extensão com id novo → o dono remove a antiga e carrega a nova.

## Migration Plan

1. Build de dev novo; primeira abertura migra `co.opensoftware.june-dev` → `dev.guilhermexp.clovy-dev`.
2. O dono concede as permissões, recola o MCP, recarrega a extensão e liga de novo a inicialização automática se usava.
3. As pastas e itens antigos ficam até o dono decidir apagá-los manualmente.
