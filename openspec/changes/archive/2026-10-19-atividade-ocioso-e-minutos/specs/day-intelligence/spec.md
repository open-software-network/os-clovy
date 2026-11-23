# Spec Delta

## MODIFIED Requirements

### Requirement: Relatório por hora
Para cada hora concluída com atividade, o sistema SHALL gerar, com uma única chamada ao provedor de atividade, um relato do que foi feito e a proporção de tempo por atividade. Os tempos exibidos MUST ser calculados pelo Clovy a partir do tempo ativo medido da hora, em segundos, usando as proporções do modelo apenas como pesos. A soma dos tempos das atividades MUST ser igual ao tempo ativo medido, e toda atividade listada MUST receber tempo maior que zero quando a hora tiver ao menos um segundo ativo por atividade. Tempo abaixo de um minuto MUST aparecer como "< 1 min".

#### Scenario: Minutos normalizados
- **WHEN** a hora teve 42 min ativos e o modelo estima 30 e 30 min para duas atividades
- **THEN** o relatório mostra 21 e 21 min
- Test: unit — normalização de tempos

#### Scenario: Hora curta com várias atividades
- **WHEN** a hora teve 2 min ativos e o modelo lista três atividades com pesos 10, 1 e 1
- **THEN** as três recebem tempo maior que zero, a soma é 2 min e as menores aparecem como "< 1 min"
- Test: unit — normalização de tempos; component — cartão da frente com atividade abaixo de um minuto

#### Scenario: Peso zero do modelo
- **WHEN** o modelo lista uma atividade com peso zero ou inválido
- **THEN** a atividade ainda recebe tempo maior que zero e a soma continua igual ao tempo medido
- Test: unit — normalização de tempos

#### Scenario: Relatório gravado antes da mudança
- **WHEN** a frente inclui um relatório antigo com minutos inteiros
- **THEN** esses minutos entram na soma da frente sem erro
- Test: unit — leitura das frentes com relatório antigo

## ADDED Requirements

### Requirement: Tempo das frentes sem zero
O tempo de cada frente e o total "ativos" da grade SHALL ser a soma dos tempos das suas atividades. Uma frente com tempo positivo abaixo de um minuto MUST aparecer como "< 1 min", nunca como "0 min".

#### Scenario: Frente de poucos segundos
- **WHEN** uma frente recebe só 40 segundos de uma hora
- **THEN** o cartão mostra "< 1 min" e o total da grade inclui os 40 segundos
- Test: component — grade de frentes com frente de 40 segundos
