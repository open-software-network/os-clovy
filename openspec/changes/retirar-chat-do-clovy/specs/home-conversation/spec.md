# Spec Delta

## ADDED Requirements

### Requirement: Dados das conversas antigas do Início são preservados
O aplicativo SHALL preservar os registros existentes das conversas, mensagens e artefatos do Início após a retirada da interface de chat, mantendo os limites de acesso existentes.

#### Scenario: Atualização com conversas existentes
- **WHEN** um perfil com conversas do Início é aberto após atualizar o Clovy
- **THEN** o app inicia em Hoje e as conversas e seus registros persistidos continuam no armazenamento do perfil

## REMOVED Requirements

### Requirement: Início responde no motor CLI padrão
### Requirement: Trabalho acontece no próprio Início
### Requirement: Conversa do Início é persistida e contínua
### Requirement: Falhas do motor no Início são acionáveis
### Requirement: Comandos de triagem do Início não existem
### Requirement: Histórico antigo do Início é preservado
