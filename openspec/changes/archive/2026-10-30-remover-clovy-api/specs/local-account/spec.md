# Spec Delta

## MODIFIED Requirements

### Requirement: Uso sem conta
O app SHALL abrir e funcionar sem login. O onboarding MUST NOT ter passo de conta, e nenhuma tela MUST bloquear o uso por falta de conta, plano ou saldo. O app MUST NOT guardar nem enviar token de servidor do Clovy.

#### Scenario: Primeira abertura de um build de release
- **WHEN** um build de release sem nenhuma variável `OS_ACCOUNTS_*` nem `OS_CLOVY_LOCAL_DEV` é aberto pela primeira vez
- **THEN** o onboarding segue sem passo de conta e termina na tela principal do app
- Test: component — onboarding e portões do app com estado inicial vazio chegam à tela principal

#### Scenario: Recursos antes presos a conta
- **WHEN** o usuário inicia ditado, prévia ao vivo, detecção de reunião ou Computer Use sem nenhuma conta
- **THEN** o recurso não falha por falta de login, plano ou liberação remota
- Test: unit — os portões de cada recurso não consultam conta nem plano

### Requirement: Nenhum serviço da Open Software
O app MUST NOT conter endereços de serviços da Open Software nem do produto June (`*.opensoftware.co`, `june.link`, `open-software-network/*`) e MUST NOT ter cobrança, indicações, telemetria P3A, envio de relatórios de erro, app companheiro, liberação remota do Computer Use nem atualizador apontando para releases de terceiros. O app MUST NOT chamar nenhum servidor Clovy API. As únicas chamadas de rede de IA são aos provedores do usuário (Groq, OpenRouter, fal, endpoints cadastrados) com a chave dele, e aos CLIs locais.

#### Scenario: Servidor padrão
- **WHEN** nenhuma `CLOVY_API_URL` está definida e o usuário dita, grava uma nota e abre o seletor de modelo
- **THEN** nenhuma requisição vai a `http://127.0.0.1:8080` nem a outro servidor do Clovy
- Test: integration — servidor sentinela em porta efêmera, injetada como a URL legada, recebe zero requisições

#### Scenario: Busca por domínios da Open Software
- **WHEN** o código-fonte do app (`src/`, `src-tauri/src/`, `src-tauri/*.json`) é varrido pela expressão `opensoftware\.co\b|june\.link|open-software-network`
- **THEN** não há ocorrência (nomes de serviço do Keychain como `co.opensoftware.clovy.*` não casam com a expressão e ficam)
- Test: script de verificação no `make verify` que falha com a lista de ocorrências

#### Scenario: Nenhuma chamada ao Clovy API
- **WHEN** o código-fonte do app é varrido por `CLOVY_API_URL`, `clovy_api_url` e pelas rotas `/v1/notes`, `/v1/dictate`, `/v1/models`, `/v1/chat/completions`, `/v1/image`, `/v1/video`, `/v1/web`, `/v1/shares`
- **THEN** não há ocorrência fora do histórico de docs e ADRs
- Test: script de verificação no `make verify`

Nota de verificação (orquestrador, 2026-10-28): os testes automatizados usam porta efêmera para evitar conflitos locais. A prova nativa em `127.0.0.1:8080` é separada, executada somente com a porta livre e limitada aos fluxos autorizados, conforme a tarefa 7.2 e seu relatório. Isso não declara sucesso dos fluxos reais de provedores adiados para validação manual pós-merge pelo dono.
