# Spec Delta

## Purpose

Expõe os dados do Clovy (notas, transcrições, ditados, memórias e atividade) a clientes MCP locais como Claude Code e Cursor, e fornece as ferramentas do Clovy aos motores CLI do chat.

## ADDED Requirements

### Requirement: Servidor MCP local opcional
O Clovy SHALL oferecer um servidor MCP por stdio, desligado por padrão, que só responde enquanto o Clovy está aberto e o servidor está ligado nas configurações. A conexão entre o processo stdio e o app MUST ser local, restrita ao usuário e autenticada por um segredo da instalação. Com o app fechado ou o servidor desligado, as ferramentas MUST responder com erro claro.

#### Scenario: Servidor desligado
- **WHEN** um cliente MCP chama uma ferramenta com o servidor desligado
- **THEN** recebe erro informando que o servidor MCP do Clovy está desligado
- Test: integration — processo stdio contra app de teste com servidor desligado

#### Scenario: Conexão de outro usuário
- **WHEN** um processo sem o segredo da instalação tenta conectar ao canal local
- **THEN** a conexão é recusada
- Test: integration — conexão sem segredo

### Requirement: Ferramentas de dados do Clovy
O servidor SHALL expor, somente leitura, ferramentas com nomes `verb_object`: `search_notes`, `get_note`, `list_dictations`, `list_memories`, e as de atividade `get_activity_timeline`, `search_activity`, `get_activity_stats`, `get_active_session`, `list_app_usage`, `get_session_detail`, `list_coding_agent_sessions`, `get_day_summary`, `get_day_panels`, `get_day_schedule`. Notas, ditados e memórias MUST vir do perfil ativo. A atividade (captura, linha do tempo, blocos de agentes, resumos) é da instalação, sem partição por perfil, como na vista "Hoje" e no agente do app. Ferramentas de atividade MUST existir apenas com a captura ligada e respeitar exclusões, retenção e limites de saída.

#### Scenario: Busca de notas pelo Claude Code
- **WHEN** o Claude Code, configurado com o servidor do Clovy, chama `search_notes` com um termo de uma nota existente
- **THEN** recebe a nota com título e trecho, do perfil ativo
- Test: integration — chamada MCP real ao servidor com banco de teste

#### Scenario: Painéis e agenda do resumo do dia
- **WHEN** um cliente MCP chama `get_day_panels` para um dia e `get_day_schedule`
- **THEN** recebe os números que o Resumo do dia desenha (foco, ocioso, ausente, categorias, apps, foco por hora, minutos por frente, reuniões da partição aberta, blocos de agentes de código das fontes ligadas, rateados no dia), sem apps e domínios excluídos nem nada antes da retenção, e o horário do resumo com as notificações e o horário de silêncio
- Test: integration — `mcp_server::tests::day_panels_draw_the_day_from_filtered_activity_meetings_and_coding_agents`, `mcp_server::tests::day_schedule_reports_the_summary_time_and_notification_settings`

### Requirement: Recursos de contexto
O servidor SHALL expor os recursos `clovy://context` (data, hora e fuso atuais) e `clovy://guide` (guia de uso das ferramentas para modelos).

#### Scenario: Contexto atual
- **WHEN** um cliente lê `clovy://context`
- **THEN** recebe data, hora e fuso do sistema
- Test: unit — handler de recursos

### Requirement: Configuração para clientes
As configurações SHALL mostrar o comando e o trecho de configuração prontos para Claude Code e Cursor, com botão de copiar, e o servidor MUST poder ser oferecido automaticamente às sessões de chat com motor CLI que aceitam MCP.

#### Scenario: Copiar configuração
- **WHEN** o usuário liga o servidor e clica em copiar a configuração do Claude Code
- **THEN** o clipboard recebe um trecho válido que aponta para o executável do servidor do Clovy instalado
- Test: unit — gerador do trecho de configuração
