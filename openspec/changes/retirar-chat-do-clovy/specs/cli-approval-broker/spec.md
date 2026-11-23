# Spec Delta

## ADDED Requirements

### Requirement: Aprovação externa visível no Clovy
O broker SHALL disponibilizar aprovações de clientes MCP externos na área de MCP do Clovy, com indicador de pendências. A interface MUST identificar ação, alvo e perfil quando aplicável, oferecer uma decisão única e registrar o resultado. O efeito MUST respeitar a autorização emitida para a chamada.

#### Scenario: Ação solicitada de fora
- **WHEN** um cliente externo pede uma ação protegida e o usuário abre o pedido
- **THEN** ele vê a ação e seu alvo e pode autorizar uma vez ou negar, sem abrir chat

#### Scenario: Pedido cancelado
- **WHEN** a chamada termina antes da decisão
- **THEN** o pedido deixa de estar acionável e nenhuma aprovação tardia produz efeito

## REMOVED Requirements

### Requirement: Pedido de aprovação visível na sessão
