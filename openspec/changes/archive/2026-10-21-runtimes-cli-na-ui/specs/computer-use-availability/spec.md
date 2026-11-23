# Spec Delta

## Purpose

Define quando o uso do computador está disponível e garante que ele não dependa de conta, plano, modelo ou serviço remoto do Clovy.

## ADDED Requirements

### Requirement: Disponibilidade só local
O uso do computador SHALL ficar "Pronto" quando o usuário o ativou, o driver local fixado está presente e as permissões do macOS (Acessibilidade e Gravação de Tela) estão concedidas. O status e a ativação MUST NOT depender de plano da conta Clovy, de modelo de visão do catálogo da Clovy API nem de resposta de `/v1/computer-use/rollout`.

#### Scenario: Sem conta Clovy
- **WHEN** o usuário sem plano Pro/Max ativa o uso do computador com driver e permissões ok
- **THEN** o status fica "Pronto"

#### Scenario: Clovy API fora do ar
- **WHEN** a Clovy API não responde
- **THEN** o status e a ativação do uso do computador não mudam

Test: integration — `status_inner` com conta sem plano, catálogo sem modelo de visão e rollout inacessível devolve `ready` quando grant, driver e permissões estão ok.

### Requirement: Só pelos CLIs com MCP do Clovy
A ferramenta `computer_use` SHALL ficar disponível apenas em turnos de CLI que recebem o `clovy-mcp`, com o vínculo de turno e a aprovação por ação existentes. O agente nativo MUST NOT oferecer a ferramenta, e nenhuma captura MUST ir a modelo da Clovy API ou OpenRouter por causa do uso do computador.

#### Scenario: Turno no Claude Code
- **WHEN** um turno assistido no Claude Code chama `computer_use`
- **THEN** o cartão de aprovação aparece e a captura volta ao Claude Code como bloco MCP

#### Scenario: Turno no agente nativo
- **WHEN** um turno roda no agente nativo
- **THEN** a ferramenta `computer_use` não está na lista de ferramentas

Test: integration — catálogo de ferramentas nativo sem `computer_use`; `clovy-mcp` com `computer_use` em turno assistido (testes existentes de `computer_use_tests` continuam verdes).
