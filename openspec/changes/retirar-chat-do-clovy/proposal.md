# Proposal

## Why

As conversas passam a acontecer nos agentes que o usuário já usa. O Clovy continua sendo o app de Atividade e a fonte de dados e ferramentas desses agentes.

## What Changes

- Retirar o chat de Início, Nova sessão, Sessões e os caminhos que abrem ou criam essas conversas. Usar Hoje como tela inicial e destino para navegação legada de chat. Preservar os dados já gravados.
- Dar aos resultados de Rotinas uma visualização sem campo de mensagem e usar a interface de Rotinas para sua configuração.
- Manter a captura, a análise e os blocos de agentes de código da Atividade, além de notas, reuniões, ditado, projetos e seus provedores.
- Permitir consultas e chamadas de ferramentas aplicáveis aos clientes MCP locais externos, com aprovação no Clovy para ações protegidas, em uma interface própria.
- Atualizar os trechos voltados a chat nas duas mudanças abertas de automação e paridade.

## Capabilities

### New Capabilities

- `desktop-navigation-without-chat` para navegação do app e execução de rotinas sem chat.

### Modified Capabilities

- `home-conversation` para aposentar a interface e preservar o histórico.
- `chat-engine-presentation` para retirar requisitos exclusivos da tela de chat.
- `clovy-mcp-server` para autorizar e decidir chamadas de clientes externos por operação.
- `cli-approval-broker` para separar as aprovações externas da tela de conversa.

## Impact

Navegação React, menus, atalhos e links nativos, chamadas de chat em notas e projetos, telas de Rotinas e Configurações, despacho MCP, aprovação e testes. O MCP usa o transporte local autenticado já distribuído. O trabalho não altera o perfil real de teste nem migra o histórico por exclusão.
