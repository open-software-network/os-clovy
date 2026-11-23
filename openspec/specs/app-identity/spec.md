# app-identity Specification

## Purpose
Define o nome e o identificador próprios do Clovy e a migração única dos dados e credenciais da identidade herdada do produto June.

## Requirements

### Requirement: Identidade própria
O app SHALL usar o identificador `dev.guilhermexp.clovy` (dev `dev.guilhermexp.clovy.dev`) e o nome "Clovy" no pacote, no executável, nos apps auxiliares, na inicialização automática, no instalador do Windows e nos nomes de credenciais e hosts de mensagens nativas. O app MUST NOT registrar o esquema `osjune://` nem usar os nomes `June`, `os-june`, `co.opensoftware.june` ou `co.opensoftware.clovy` fora da leitura de origem da migração.

#### Scenario: Pacote instalado
- **WHEN** o build de release do Mac e o instalador do Windows são gerados
- **THEN** o Mac tem `Clovy.app` com `CFBundleIdentifier` `dev.guilhermexp.clovy` e executável `clovy`, e o Windows instala `clovy.exe` com editor do dono
- Test: script de verificação dos artefatos (`verify-macos-app-name.mjs`, `verify-windows-artifacts.ps1`)

### Requirement: Migração única da identidade June
Na primeira abertura do build dev com a identidade nova no macOS, quando a pasta de dados nova não existir ou estiver vazia e a antiga existir, o app SHALL copiar, antes de abrir qualquer banco, as pastas de dados e de configuração dev antigas (`co.opensoftware.june-dev` para `dev.guilhermexp.clovy-dev`), o armazenamento do WebView e os itens de credencial para os nomes novos. O build de release MUST NOT migrar: abre sempre com o perfil próprio e nunca lê `co.opensoftware.june` (decisão do dono: o release começa vazio). No Windows o app MUST NOT migrar: sempre começa com perfil novo (o dono não tem perfil June lá).

#### Scenario: Windows não migra
- **WHEN** o app abre no Windows
- **THEN** nenhuma migração roda e o perfil começa vazio
- Test: unit — `the_migration_runs_only_on_macos` (módulo e chamada só sob `cfg(target_os = "macos")`, sem código só-Windows)

#### Scenario: Release do Mac não migra
- **WHEN** o build de release abre no macOS e existe `co.opensoftware.june` com dados
- **THEN** nenhuma migração roda, nada é lido da pasta nem das credenciais antigas e o app abre com o perfil `dev.guilhermexp.clovy` vazio
- Test: unit — `the_release_build_never_migrates` (a chamada só existe em build dev; a origem de produção não aparece no código de release)

#### Scenario: Sessões de agente pausadas
- **WHEN** o perfil antigo tem uma execução de agente esperando o usuário, com estado serializado
- **THEN** depois da migração nenhuma execução, sessão ou cartão de interrupção fica retomável e nenhum `interrupted_state_json` sobra; uma execução concluída mantém o resultado
- Test: integration — `no_agent_run_stays_resumable_after_the_migration`

#### Scenario: Caminhos da importação Hermes e do Obsidian
- **WHEN** `hermes/config.yaml`, `hermes/cron/jobs.json` ou `obsidian.json` guardam um caminho dentro do perfil antigo
- **THEN** o comando e os argumentos de MCP, o `script` e o `workdir` dos jobs e o `vaultPath` passam a apontar para o perfil novo; um valor fora do perfil, um irmão parecido ou um programa inline fica igual
- Test: integration — `hermes_import_paths_and_the_obsidian_vault_are_rewritten`, `an_obsidian_vault_outside_the_profile_stays`

#### Scenario: Migração com dados existentes
- **WHEN** existe `co.opensoftware.june-dev` com `notes.sqlite3`, `activity.sqlite3` e a chave SQLCipher no Keychain, e o app dev novo abre pela primeira vez
- **THEN** o app abre com as mesmas notas e a mesma atividade, lendo o banco de atividade com a chave copiada para o serviço novo
- Test: integration — diretórios temporários com fixture de pasta antiga e armazenamento de credenciais falso; conferência de conteúdo e de que a origem não mudou

#### Scenario: Pasta nova já em uso
- **WHEN** a pasta nova já contém dados
- **THEN** nenhuma cópia acontece
- Test: unit — decisão de migrar com pasta nova vazia, ausente e preenchida

### Requirement: Migração atômica e sem perda
A migração MUST ser completa ou não acontecer: se falhar, o app MUST deixar a pasta nova vazia, registrar o erro e não apagar nada. As pastas e itens antigos MUST ficar intactos em qualquer caso.

#### Scenario: Falha no meio da cópia
- **WHEN** a cópia falha depois de copiar parte dos arquivos
- **THEN** a pasta nova volta a ficar vazia, o erro é registrado e a pasta antiga continua igual
- Test: integration — falha injetada na cópia

#### Scenario: Rollback interrompido
- **WHEN** a publicação falha e a limpeza do que foi publicado não termina
- **THEN** o rollback já está gravado no journal antes de qualquer remoção; a abertura seguinte termina o rollback ou, se ainda não conseguir, bloqueia; os restos nunca contam como migração concluída
- Test: integration — `an_interrupted_rollback_blocks_the_next_launch_until_it_finishes`
