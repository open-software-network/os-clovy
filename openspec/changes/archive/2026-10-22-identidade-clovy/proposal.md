# Proposal

## Why

O app ainda se identifica como o produto June da Open Software, empresa de onde veio o fork:

- identificador `co.opensoftware.june` (`tauri.conf.json:5`), do qual derivam as pastas de dados (`~/Library/Application Support/co.opensoftware.june{,-dev}`, `%APPDATA%\…`), as permissões do macOS e a assinatura exigida do driver de Computer Use;
- `June.app`, `CFBundleDisplayName` "June", executável `os-june`, apps auxiliares "June …" (`June Computer Use Driver.app`, `June Dictation Helper.app`, `june-system-audio-recorder`, `june-dictation-helper`, `june-nm-shim`), inicialização automática "June" (`lib.rs:164-167`), editor "Open Software Network" (`tauri.windows.conf.json:4`), `authors` "Open Software Network";
- serviços do Keychain/Gerenciador de Credenciais `co.opensoftware.clovy*` e `co.opensoftware.june*`, hosts de mensagens nativas `co.opensoftware.clovy.extension` e `june.extension`;
- camadas de compatibilidade June→Clovy (`env_compat.rs` com `OS_JUNE_*`, `storage-compat.ts`, alias `june-nm-shim`, credenciais legadas em `credential_compat.rs`);
- extensão do navegador presa ao item da Chrome Web Store da Open Software (id `jfpogffllplkfoooiaibjkojkngbdnik`, derivado da `key` do `manifest.json`).

O dono decidiu trocar o identificador e apagar esses vestígios.

## What Changes

- **BREAKING** Identificador `dev.guilhermexp.clovy` (dev: `dev.guilhermexp.clovy.dev`, perfil de dev `dev.guilhermexp.clovy-dev`) e apps auxiliares `dev.guilhermexp.clovy.<helper>`.
- **BREAKING** Nome "Clovy" em tudo que é visível ou registrado: `Clovy.app`, executável `clovy`, apps e binários auxiliares `Clovy …`/`clovy-…`, inicialização automática "Clovy", esquema de URL só `clovy://`, editor e `authors` com o nome do dono.
- Migração única na primeira abertura: copia dados e configurações das pastas antigas para as novas (só no Mac, produção e dev; no Windows o app começa limpo), o armazenamento do WebView e os itens do Keychain para os nomes novos, incluindo a chave SQLCipher do banco de atividade. As pastas e itens antigos ficam intactos.
- Serviços de credencial e hosts de mensagens nativas com o prefixo novo; a extensão ganha chave própria (id novo) e é carregada sem passar pela loja da Open Software.
- Saem as camadas de compatibilidade June, exceto a leitura única de origem da migração.
- Docs, ADR (substituindo a ADR-0055 de identidade imutável), scripts de verificação e testes atualizados.

## Capabilities

### New Capabilities
- `app-identity`: nome, identificador e migração única dos dados e credenciais do app.

### Modified Capabilities

## Impact

- Config Tauri (`tauri.conf.json`, `tauri.macos.conf.json`, `tauri.windows.conf.json`, `Info.plist`), `Cargo.toml` (`[[bin]]`, `default-run`, `authors`), `build.rs`, `cua-driver-pin.json`, `native/mac-dictation-helper/main.swift`, `computer_use_driver.rs`, `computer_use.rs`, `extension_host.rs`, `activity/filter.rs`, `activity/key.rs`, `llm/secrets.rs`, `credential_compat.rs` e usuários, `browser/launcher.rs`, `app_paths.rs`, `lib.rs`, `env_compat.rs`, `system_macos.rs`, `extension/` (manifest e protocolo), `src/lib/storage-compat.ts`.
- Scripts: `dev-app-identity.mjs`, `dev-app-bundle.mjs`, `verify-macos-app-name.mjs`, `bundle-nm-shim.sh`, `build-windows.ps1`, `verify-windows-artifacts.ps1`, `computer-use-*.mjs`, `validate-extension-id.mjs`.
- `clovy-api`: `apns_bundle_id` sai junto do companheiro (feito na change de serviços) ou recebe o id novo.
- `AGENTS.md` (regras sobre `co.opensoftware.june-dev`), `CONTEXT.md`, docs e ADRs.
- O dono precisa conceder de novo as permissões do macOS ao app novo e aos auxiliares, recolar a configuração MCP nos clientes e recarregar a extensão.
- Depende de `desvincular-servicos-open-software` (mexe em `tauri.conf.json`, `lib.rs` e esquema de URL).
