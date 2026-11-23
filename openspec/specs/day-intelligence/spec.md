# day-intelligence Specification

## Purpose
Gera, a partir da atividade, das sessões de agentes e das reuniões do Clovy, relatórios por hora, frentes de trabalho e um resumo do dia com standup, entregues no horário e avisados por notificação.

## Requirements

### Requirement: Destilação local antes do modelo
Antes de qualquer chamada de modelo, o texto capturado de cada sessão SHALL passar por uma destilação local que remove interface repetida e lixo, deduplica trechos lexical e semanticamente (com embeddings calculados localmente), seleciona trechos representativos e preserva entidades (identificadores de tickets, hashes, números de PR, caminhos de arquivo). A destilação MUST não usar rede, exceto para baixar o modelo de embeddings uma única vez.

#### Scenario: Entidades preservadas
- **WHEN** a sessão contém linhas repetidas de interface e uma linha com `KAN-123` e um caminho de arquivo
- **THEN** o texto destilado não contém as linhas repetidas e contém `KAN-123` e o caminho
- Test: unit — pipeline de destilação com texto sintético

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

### Requirement: Frentes de trabalho do dia
O sistema SHALL agrupar os relatórios por hora em frentes de trabalho do dia, incorporando cada novo relatório às frentes existentes (atribuindo a uma frente ou criando outra) sem reescrever frentes já registradas.

#### Scenario: Hora atribuída a frente existente
- **WHEN** existe a frente "Corrigir bug de login" e a nova hora continua esse trabalho
- **THEN** a hora é anexada a essa frente e as demais frentes não mudam
- Test: unit — dobra incremental com resposta de modelo simulada

### Requirement: Resumo do dia e standup
O sistema SHALL gerar o resumo do dia com narrativa, insights, standup (feito, em andamento, bloqueios) e painéis gráficos, incluindo as reuniões e notas gravadas no Clovy naquele dia e os blocos de agentes de código. Números exibidos nos painéis MUST vir dos dados gravados no momento da exibição, não do texto do modelo. O texto MUST seguir o idioma da interface; as chaves do JSON trocado com o modelo MUST permanecer em inglês.

#### Scenario: Resumo em português
- **WHEN** a interface está em pt-BR e o resumo do dia é gerado
- **THEN** narrativa e standup aparecem em português e os totais dos painéis batem com as estatísticas do dia
- Test: integration — geração com provedor falso e conferência dos totais

#### Scenario: Reunião incluída
- **WHEN** o usuário gravou uma reunião no Clovy às 15:00
- **THEN** o resumo do dia menciona a reunião pelo título da nota
- Test: integration — montagem do contexto do resumo com nota de reunião de teste

### Requirement: Agendamento com recuperação
O resumo do dia SHALL ser gerado automaticamente uma vez por dia no horário configurado (padrão 18:00) e, se o Mac estava suspenso nesse horário, ao acordar. O usuário MUST poder gerar novamente sob demanda. Sem provedor de atividade, o agendamento MUST ficar inativo.

#### Scenario: Mac dormindo às 18:00
- **WHEN** o Mac dorme das 17:30 às 19:00
- **THEN** o resumo é gerado logo após acordar e apenas uma vez naquele dia
- Test: unit — agendador com relógio simulado

### Requirement: Notificações
O sistema SHALL notificar quando o resumo do dia estiver pronto e quando houver falha do sistema de atividade, com ação para abrir a vista correspondente e, quando a plataforma permitir ações, "Adiar 1h". Notificações MUST respeitar o horário silencioso configurado e a chave geral de notificações.

#### Scenario: Horário silencioso
- **WHEN** o horário silencioso é 22:00–08:00 e o resumo fica pronto às 23:00
- **THEN** a notificação só é entregue às 08:00
- Test: unit — política de entrega com relógio simulado

#### Scenario: Abrir pela notificação
- **WHEN** o usuário clica na notificação de resumo pronto
- **THEN** o Clovy abre a vista "Hoje" no resumo daquele dia
- Test: e2e — app de desenvolvimento, notificação real e captura da vista aberta

### Requirement: Vista do resumo
A vista "Hoje" SHALL exibir o resumo do dia, as frentes de trabalho e os relatórios por hora, com cópia do standup em um clique e estado claro quando o recurso estiver desligado por falta de provedor.

#### Scenario: Copiar standup
- **WHEN** o usuário clica em "Copiar standup"
- **THEN** o texto do standup vai para o clipboard formatado em tópicos
- Test: unit — componente do resumo com dados de teste

### Requirement: Tempo das frentes sem zero
O tempo de cada frente e o total "ativos" da grade SHALL ser a soma dos tempos das suas atividades. Uma frente com tempo positivo abaixo de um minuto MUST aparecer como "< 1 min", nunca como "0 min".

#### Scenario: Frente de poucos segundos
- **WHEN** uma frente recebe só 40 segundos de uma hora
- **THEN** o cartão mostra "< 1 min" e o total da grade inclui os 40 segundos
- Test: component — grade de frentes com frente de 40 segundos
