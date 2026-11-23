## MODIFIED Requirements

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
