# Spec Delta

## ADDED Requirements

### Requirement: Contexto e guia correspondem à autoridade real
Turno vinculado SHALL receber contexto estruturado de conversa/run/engine/superfície/projeto/partição/workspace e políticas efetiva/configurada obtidos pelo host, sem tokens/segredos. Cliente externo recebe só contexto autorizado, conforme `retirar-chat-do-clovy`. Guia e catálogo SHALL distinguir superfícies, leituras, mutações autorizadas, custos e aprovações.

#### Scenario: Contexto externo e bound
- **WHEN** externo e turno vinculado pedem contexto/guia
- **THEN** externo recebe somente sua autoridade atual e o turno vinculado recebe sua identidade e política do host, mesmo após troca de partição na UI
- Test: integration — recursos/despacho e escopo após espera

### Requirement: Semântica temporal e erros preservados
As ferramentas SHALL explicar que intervalos de atividade retornam registros completos sobrepostos com totais recortados e `active` é estado materializado, não watermark instantâneo. Falhas MUST preservar origem/código/classificação/disposição de entrega disponível; consulta de status MUST NOT ativar recurso/login/controle.

#### Scenario: Gap sobreposto e recurso desligado
- **WHEN** consulta retorna gap iniciado antes da janela ou status de recurso desligado
- **THEN** limites originais e totais recortados são claros, e nenhuma permissão é concedida
- Test: integration — fixture temporal e leitura sem efeito
