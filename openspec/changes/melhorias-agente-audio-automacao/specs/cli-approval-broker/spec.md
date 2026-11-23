# Spec Delta

## MODIFIED Requirements

### Requirement: Decisão única
O broker SHALL aceitar uma única decisão por pedido. Aprovação `once` libera a chamada; negação devolve `user_denied`. Escopos `session` e `always` MUST ser recusados explicitamente enquanto não implementados, nunca reduzidos silenciosamente a `once`. Segunda decisão MUST falhar sem efeito.

#### Scenario: Duas respostas simultâneas
- **WHEN** duas respostas concorrem pelo mesmo pedido
- **THEN** só uma decisão libera a ferramenta
- Test: integration — resolução concorrente e uma mutação

#### Scenario: Escopo não implementado
- **WHEN** cliente envia `session` ou `always`
- **THEN** recebe erro de resolução inválida e a ação não é liberada
- Test: unit — escopo recusado

## ADDED Requirements

### Requirement: Política verificável por execução
O host SHALL capturar a política efetiva no início de cada execução interna e aplicar mudanças de configuração somente à próxima execução. O campo legado `approval_mode` MUST NOT conceder autoridade. Qualquer autorização por política já prevista MUST ser registrada antes do efeito em recibo correlacionado à execução e à operação. Ações externas seguem o contrato de aprovação por chamada de `retirar-chat-do-clovy`.

#### Scenario: Configuração alterada durante a execução
- **WHEN** a configuração é alterada enquanto uma rotina está em andamento
- **THEN** a execução atual conserva a política capturada e a próxima usa a configuração atualizada
- Test: integração com snapshot, repositório e broker

#### Scenario: Origem do recibo verificável
- **WHEN** uma política existente autoriza uma ação interna
- **THEN** o recibo identifica a origem da autorização e a operação exata, e falha ao persistir impede o efeito
- Test: integração com recibo antes do despacho e falha de armazenamento

### Requirement: Bypass não concede capacidades
Bypass SHALL atuar apenas na camada de pedidos do broker, preservando clarificações, restrições reais do CLI, capabilities, app/janela/aba, campos sensíveis, autenticação, recibos consumíveis e permissões do SO. Cancelamento/revogação MUST bloquear novos efeitos; entrega incerta MUST NOT ser repetida automaticamente.

#### Scenario: Clarificação e recurso desligado
- **WHEN** run em bypass pede esclarecimento ou usa recurso desabilitado
- **THEN** pergunta continua humana e recurso permanece bloqueado
- Test: integração com consumidores internos aplicáveis, rotina e cliente externo sem bypass e recibo de conector não reutilizável
