# Spec Delta

## ADDED Requirements

### Requirement: Teste automático ao escolher provedor de Atividade
Ao escolher para Atividade uma CLI instalada, com modo one-shot e sem nível de saída estruturada medido, o app SHALL rodar o teste do provedor antes de recusar a escolha e SHALL aplicá-la se o nível medido suportar JSON schema. Falhas MUST ser explicadas pela causa real: login ausente aponta "Autenticar no Clovy"; cota ou prazo esgotado têm mensagem própria; só o nível insuficiente mostra a mensagem de JSON schema.

#### Scenario: Antigravity autenticado sem teste anterior
- **WHEN** o usuário escolhe Antigravity em Atividade e o perfil do Clovy está autenticado, sem nível medido
- **THEN** o app mostra que está testando, grava o nível medido e aplica a escolha

#### Scenario: Antigravity sem login no perfil do Clovy
- **WHEN** o teste automático falha por falta de autenticação
- **THEN** a escolha não é aplicada e a mensagem pede "Autenticar no Clovy", sem falar de JSON schema
