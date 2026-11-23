# Spec Delta

## Purpose

Preservar inferência textual remota por OpenRouter e a escolha dos motores CLI/endpoints existentes sem dependência ativa de Venice.

## ADDED Requirements

### Requirement: Texto remoto OpenRouter
O sistema SHALL executar chat via API, geração de notas, limpeza de ditado e demais consumidores de inferência textual por OpenRouter, usando modelos reais e preservando os resultados de cada fluxo.

#### Scenario: Chat remoto e ferramentas
- **WHEN** o usuário seleciona um modelo OpenRouter compatível e envia mensagens ou uma ação com ferramentas
- **THEN** recebe resposta/stream e tool calls no contrato existente, com erros e finalização corretos, sem rota Venice
- Test: integration — boundary de JSON/SSE/tool calls e erros; prova externa e UI apenas com configuração e custo autorizados.

#### Scenario: Nota e limpeza de ditado
- **WHEN** o usuário solicita geração de nota ou limpeza textual com a rota remota selecionada
- **THEN** OpenRouter produz o resultado pertinente sem trocar o áudio Groq, sem perder histórico e sem consumir modo CLI escolhido
- Test: integration — serviços de nota/cleanup com sucesso, erro e metering; prova nativa/external dos dois fluxos em sandbox autorizado.

### Requirement: Catálogo e capacidades reais
O sistema SHALL apresentar modelos, capacidades, limites e preços provenientes do catálogo OpenRouter e SHALL resolver a seleção automática apenas para modelos que satisfaçam os requisitos da ação.

#### Scenario: Seleção com visão ou ferramentas
- **WHEN** a ação precisa de visão ou tool calling
- **THEN** o seletor usa capacidades reais do catálogo e não oferece ou roteia um modelo incompatível por inferência baseada em nome/traits
- Test: integration — parser do catálogo e resolução com casos positivos/negativos de modalities e supported_parameters.

### Requirement: Continuidade dos motores existentes
O sistema SHALL preservar motores CLI/endpoints, sua seleção e seus segredos, sem converter o chat Claude existente para OpenRouter durante a migração.

#### Scenario: Perfil com Claude CLI
- **WHEN** um perfil com chat Claude e atividade Claude recebe a atualização
- **THEN** essas escolhas continuam válidas e os usos remotos legados de notas/cleanup passam para OpenRouter sem regravar chaves de endpoint
- Test: unit — migração de settings e registro LLM usando stores isolados, incluindo usos mistos e overrides.

### Requirement: Privacidade textual verdadeira
O sistema SHALL apresentar a política efetiva do novo provedor sem herdar afirmações Venice de TEE, rota privada ou zero retention que OpenRouter não comprove.

#### Scenario: Modelo sem garantia de rota privada
- **WHEN** um modelo OpenRouter não tem a garantia exibida anteriormente por Venice
- **THEN** o UI/contrato não o rotula como TEE ou zero retention garantidos e mantém a escolha e consentimento pertinentes
- Test: integration — catálogo/privacy normalizado e UI com propriedades reais; revisão de headers e documentação.
