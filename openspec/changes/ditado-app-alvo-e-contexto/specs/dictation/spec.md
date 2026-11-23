# Spec Delta

## ADDED Requirements

### Requirement: Histórico mostra o app de destino
Cada ditado gravado no histórico SHALL guardar o app de destino fixado no fim da gravação, quando conhecido. O histórico do Ditado MUST mostrar o ícone desse app em cada linha e no diálogo do item, obtido pelo mesmo mecanismo do HUD. Sem app conhecido ou sem ícone disponível, a linha MUST mostrar o ícone de microfone.

#### Scenario: Ditado no Slack
- **WHEN** o usuário dita com o Slack como destino e abre o histórico
- **THEN** a linha desse ditado mostra o ícone do Slack
- Test: unit — gravação do histórico com bundle id; component — linha com `appBundleId` renderiza o ícone do app

#### Scenario: Destino trocado depois da gravação
- **WHEN** o usuário para a gravação no Slack e muda de app antes de a transcrição terminar
- **THEN** o histórico guarda o Slack, o destino fixado no fim da gravação
- Test: unit — o bundle id gravado vem do evento de fim de gravação

#### Scenario: Ditado antigo
- **WHEN** o histórico contém um ditado gravado antes desta mudança
- **THEN** a linha mostra o ícone de microfone
- Test: unit — migração mantém linhas antigas com app nulo; component — linha sem `appBundleId`

#### Scenario: App sem ícone
- **WHEN** o app de destino foi desinstalado e o ícone não é encontrado
- **THEN** a linha mostra o ícone de microfone
- Test: component — falha do ícone cai no microfone
