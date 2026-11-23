# Spec Delta

## Purpose

Define a navegação do Clovy como aplicativo de Atividade, notas, reuniões, projetos, ditado e rotinas, com as conversas feitas por clientes externos.

## ADDED Requirements

### Requirement: Interface sem chat interno
O Clovy SHALL abrir em Hoje e apresentar suas áreas de Atividade, notas de reunião, projetos, ditado, rotinas e configurações. O shell, menu nativo, bandeja, atalhos, busca, abas, notificações, notas e projetos MUST deixar de oferecer abertura ou criação de chat. Navegação legada de chat SHALL redirecionar para Hoje sem iniciar um turno.

#### Scenario: Abre e navega
- **WHEN** o usuário abre o app, abre uma aba ou segue um destino legado de chat
- **THEN** ele alcança uma vista suportada do app, sem composer, conversa ou início de turno

### Requirement: Atividade e rotinas disponíveis
Captura, classificação, resumos, linha do tempo, provedores e leitura de agentes de código da Atividade SHALL continuar disponíveis. A configuração, execução e consulta de resultados de rotinas MUST funcionar em Rotinas sem interface de conversa.

#### Scenario: Vê resultado de rotina
- **WHEN** o usuário abre uma execução no histórico de Rotinas
- **THEN** vê o estado e resultado ou o erro em uma visualização sem campo de envio
