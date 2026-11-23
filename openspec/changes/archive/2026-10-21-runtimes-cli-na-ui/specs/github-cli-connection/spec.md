# Spec Delta

## Purpose

Define a conexão automática do conector GitHub pela sessão do GitHub CLI (`gh`) já logada no Mac.

## ADDED Requirements

### Requirement: Conexão automática pelo gh
Quando não há conta GitHub conectada, o `gh` está instalado (achado pelo PATH do shell de login) e `gh auth status` mostra login em `github.com`, o Clovy SHALL conectar sozinho uma conta "GitHub (via gh)" com a identidade de `GET https://api.github.com/user`, com o pacote de leitura ativo. Isso MUST acontecer sem clique do usuário ao abrir o app ou a tela de Plugins.

#### Scenario: gh logado e nenhuma conta
- **WHEN** o usuário abre Plugins com o `gh` logado e nenhuma conta GitHub conectada
- **THEN** a linha do GitHub mostra a conta do `gh` conectada, marcada "via gh", com leitura ativa

#### Scenario: Já existe conta pela GitHub App
- **WHEN** já há uma conta conectada pelo device flow
- **THEN** o Clovy não cria a conta via `gh`

#### Scenario: gh ausente ou deslogado
- **WHEN** o `gh` não existe ou não está logado
- **THEN** nada muda e o device flow continua disponível

Test: integration — `gh` falso no PATH (script que imprime status/token) e servidor HTTP falso no lugar de `api.github.com`: conecta com gh logado; não conecta com conta da App existente; não conecta sem `gh`.

### Requirement: Token do gh lido a cada chamada, nunca guardado
O token da conta via `gh` SHALL ser obtido de `gh auth token --hostname github.com` em cada chamada à API do GitHub e MUST NOT ser gravado no Keychain, no SQLite, em arquivo ou em log, nem passado a argv, ambiente ou arquivo de um CLI de agente. Se o `gh` deixar de devolver token, a conta MUST aparecer como "reconectar" e as ferramentas do GitHub MUST falhar com erro explicável.

#### Scenario: Chamada de ferramenta
- **WHEN** uma ferramenta do GitHub roda com a conta via `gh`
- **THEN** o token vem do `gh` naquela hora e vai só para `api.github.com`

#### Scenario: gh deslogado depois
- **WHEN** o usuário faz `gh auth logout`
- **THEN** a próxima chamada falha com aviso de reconectar e nada do token antigo fica no Clovy

Test: integration — `gh` falso conta invocações; nenhuma escrita no armazenamento de tokens; logout simulado leva a `reconnect_required`.

### Requirement: Permissões do Clovy continuam valendo
A conta via `gh` SHALL obedecer aos mesmos pacotes `github_read` e `github_write` e à aprovação de escrita do conector, independentemente dos escopos do token do `gh`. A escrita MUST começar desligada.

#### Scenario: Escrita sem pacote
- **WHEN** um turno tenta `create_issue` com a conta via `gh` sem `github_write`
- **THEN** a chamada é recusada como hoje para a conta da App

Test: integration — dispatch de `create_issue` recusado sem `github_write` e aprovado com o pacote mais a aprovação.

### Requirement: Desconectar desliga a reconexão automática
Desconectar a conta via `gh` SHALL gravar uma preferência local que impede a conexão automática até o usuário conectar de novo por ação explícita.

#### Scenario: Desconectar
- **WHEN** o usuário desconecta "GitHub (via gh)" e reabre Plugins
- **THEN** a conta não volta sozinha e há um botão para conectar pelo `gh` de novo

Test: integration — após desconectar, a detecção com `gh` logado não cria conta; a ação explícita cria.
