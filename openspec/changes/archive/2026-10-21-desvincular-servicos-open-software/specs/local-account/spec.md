## Purpose
Garante que o app funciona só com identidade local, sem conta, cobrança, telemetria ou atualizador de terceiros, falando apenas com servidores configurados pelo dono.

## ADDED Requirements

### Requirement: Uso sem conta
O app SHALL abrir e funcionar sem login. O onboarding MUST NOT ter passo de conta, nenhuma tela MUST bloquear o uso por falta de conta, plano ou saldo, e as chamadas ao `clovy-api` MUST usar o token local configurado (`OS_CLOVY_LOCAL_DEV_BEARER_TOKEN`, padrão `local-dev-token`).

#### Scenario: Primeira abertura de um build de release
- **WHEN** um build de release sem nenhuma variável `OS_ACCOUNTS_*` nem `OS_CLOVY_LOCAL_DEV` é aberto pela primeira vez
- **THEN** o onboarding segue sem passo de conta e termina na tela principal do app
- Test: component — onboarding e portões do app com estado inicial vazio chegam à tela principal

#### Scenario: Recursos antes presos a conta
- **WHEN** o usuário inicia ditado, prévia ao vivo, detecção de reunião ou Computer Use sem nenhuma conta
- **THEN** o recurso não falha por falta de login, plano ou liberação remota
- Test: unit — os portões de cada recurso não consultam conta nem plano

### Requirement: Nenhum serviço da Open Software
O app MUST NOT conter endereços de serviços da Open Software nem do produto June (`*.opensoftware.co`, `june.link`, `open-software-network/*`) e MUST NOT ter cobrança, indicações, telemetria P3A, envio de relatórios de erro, app companheiro, liberação remota do Computer Use nem atualizador apontando para releases de terceiros. O endereço padrão do `clovy-api` SHALL ser `http://127.0.0.1:8080`, sobreposto por `CLOVY_API_URL`, e links de compartilhamento SHALL usar a origem desse servidor.

#### Scenario: Servidor padrão
- **WHEN** nenhuma `CLOVY_API_URL` está definida e o app chama o `clovy-api`
- **THEN** a requisição vai a `http://127.0.0.1:8080`
- Test: unit — resolução do endereço sem variável e com variável

#### Scenario: Busca por domínios da Open Software
- **WHEN** o código-fonte do app (`src/`, `src-tauri/src/`, `src-tauri/*.json`) é varrido pela expressão `opensoftware\.co\b|june\.link|open-software-network`
- **THEN** não há ocorrência (nomes de serviço do Keychain como `co.opensoftware.clovy.*` não casam com a expressão e ficam)
- Test: script de verificação no `make verify` que falha com a lista de ocorrências
