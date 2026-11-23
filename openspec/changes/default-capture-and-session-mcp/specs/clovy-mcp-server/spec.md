# Spec Delta

## Purpose

Define o servidor MCP local disponível automaticamente nas sessões compatíveis do Clovy, mantendo desligamento persistente e autenticação local.

## ADDED Requirements

### Requirement: MCP local iniciado por padrão
O Clovy SHALL iniciar seu servidor MCP ao abrir sem ativação manual para configuração nova ou campos ausentes. Sessões CLI compatíveis MUST receber as ferramentas locais quando o listener estiver pronto. A conexão MUST permanecer local, restrita ao usuário e autenticada. Clientes externos MUST continuar exigindo sua própria configuração existente.

#### Scenario: Sessão pronta sem ligar MCP
- **WHEN** o Clovy abre com configuração nova e inicia uma sessão CLI compatível
- **THEN** o listener responde ao protocolo MCP e a sessão recebe as ferramentas locais sem visitar Configurações
- Test: integration — conexão MCP real e contrato de lançamento CLI; smoke nativo com tools/list pelo stdio

### Requirement: Migração única e opt-out durável do MCP
Na primeira abertura após esta atualização, o Clovy SHALL ativar o MCP dos perfis legados uma única vez, inclusive `false` antigo. A atualização MUST ser gravada na abertura, antes de qualquer nova escolha; desligamento posterior salvo MUST sobreviver a reinícios e a gravações de configurações de captura. Exceção: se a gravação da atualização falhar, o Clovy MUST registrar a falha e seguir com o MCP ligado só em memória, sem declarar a atualização persistida (a próxima abertura a repete). Desligar o MCP sem conseguir gravar MUST retornar erro e manter o estado anterior; a primeira gravação bem-sucedida de qualquer switch MUST persistir a atualização junto com a escolha.

#### Scenario: Perfil legado sem MCP
- **WHEN** um perfil anterior com MCP desligado abre e reinicia
- **THEN** o listener sobe em ambas as aberturas sem ativação manual
- Test: integration — configuração temporária, ciclo de vida real do listener e recarga

#### Scenario: Desligamento após atualização
- **WHEN** o usuário desliga o MCP e reinicia duas vezes
- **THEN** o listener permanece desligado e o cliente recebe erro claro
- Test: integration — persistência e conexão real; smoke nativo por UI e reinício

#### Scenario: Falha ao gravar a atualização
- **WHEN** a atualização de um perfil legado não pode ser gravada e o usuário desliga o MCP primeiro sem conseguir gravar e depois com gravação bem-sucedida
- **THEN** a tentativa falha retorna erro e mantém o MCP ligado, e o desligamento salvo sobrevive a dois reinícios sem nova atualização
- Test: integration — diretório de configuração sem escrita, verificação do arquivo durável e recarga real

### Requirement: Controles independentes
Desligar captura MUST preservar MCP de notas, ditados e memórias; suas ferramentas de atividade MUST seguir a regra de exposição existente. Desligar MCP MUST preservar a preferência de captura. Iniciar sem permissão de captura MUST não bloquear o MCP local.

#### Scenario: Captura desligada com MCP ligado
- **WHEN** o usuário desliga apenas captura
- **THEN** MCP segue respondendo às ferramentas de notas, sem anunciar ferramentas de atividade
- Test: integration — tools/list real com configuração persistida

#### Scenario: MCP desligado com captura ligada
- **WHEN** o usuário desliga apenas MCP e reinicia
- **THEN** captura continua ligada e o listener permanece desligado
- Test: integration — carga de preferências e ciclo de vida dos dois serviços
