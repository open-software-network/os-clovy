# Spec Delta

## ADDED Requirements

### Requirement: Ditados como contexto do período
O relatório por hora e o resumo do dia SHALL incluir, como seção dos dados enviados ao modelo, os ditados do perfil ativo feitos no período, em ordem cronológica, com horário local, app de destino quando conhecido e trecho limitado do texto. A seção MUST ficar dentro da cerca de dados e MUST respeitar limites de quantidade e de tamanho. Sem ditados no período, o prompt MUST ser idêntico ao de antes.

#### Scenario: Hora com ditado
- **WHEN** a hora das 14:00 teve atividade e um ditado às 14:32 no Slack
- **THEN** o prompt do relatório dessa hora contém a linha do ditado com 14:32, o app e o texto, dentro da cerca de dados
- Test: integration — pipeline com fontes falsas que devolvem ditados

#### Scenario: Hora sem ditado
- **WHEN** a hora teve atividade e nenhum ditado
- **THEN** o prompt é igual, byte a byte, ao gerado sem a seção de ditados
- Test: unit — prompt com lista vazia de ditados

#### Scenario: Ditado sem atividade
- **WHEN** a hora teve ditado mas zero minutos ativos
- **THEN** nenhum relatório é gerado e o provedor não é chamado
- Test: integration — pipeline com ditado e hora sem atividade

#### Scenario: Dia só com ditados
- **WHEN** o dia não tem relatórios, reuniões nem blocos de agentes, mas tem ditados
- **THEN** nenhum resumo do dia é gerado e o provedor não é chamado
- Test: integration — pipeline com dia vazio e ditados

#### Scenario: Texto tentando fechar a cerca
- **WHEN** o texto de um ditado contém a tag de fechamento da cerca de dados
- **THEN** a tag é escapada e a seção continua dentro da cerca
- Test: unit — prompt com ditado contendo a tag de fechamento

#### Scenario: Muitos ditados
- **WHEN** a hora tem mais ditados que o limite ou textos longos
- **THEN** o prompt traz no máximo o limite de ditados, cada um cortado no limite de caracteres
- Test: unit — prompt com ditados acima dos limites
