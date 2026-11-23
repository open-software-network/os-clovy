# Spec Delta

## Purpose

Apresentar frentes e retomadas em um calendário legível que aproveita a altura disponível, com cartões compactos e sem sobreposição física, preservando os dados temporais e os demais fluxos.

## ADDED Requirements

### Requirement: Escala adaptativa do calendário
O calendário SHALL adaptar a escala temporal das horas ativas à altura útil disponível, com crescimento limitado em dias curtos e rolagem em dias longos. Redimensionar a janela MUST atualizar a geometria sem alterar horários ou duração ativa.

#### Scenario: Dia curto em painel alto
- **WHEN** duas frentes ocupam 09:00–09:10 e 09:30–09:40 num calendário com 600 pixels úteis
- **THEN** a escala aproveita a altura disponível, sem esticar indefinidamente os cartões, e os dois intervalos continuam posicionados no horário correto
- Test: integration — comparação de geometria com entradas e viewport fixos, seguida de smoke visual.

#### Scenario: Janela redimensionada e dia longo
- **WHEN** o usuário aumenta ou reduz a altura disponível ou abre um dia que excede a janela
- **THEN** a grade se recalcula sem ciclos de resize, dados cortados ou perda da rolagem; a escala não reduz a leitura abaixo do limite previsto
- Test: integration — redimensionamento no navegador e observação de geometria estável.

### Requirement: Conteúdo progressivo em cartões compactos
Cartões com pouco espaço SHALL apresentar uma forma compacta legível. Metadados e notas SHALL aparecer progressivamente conforme houver espaço, sem cortar linhas ao meio. O nome completo, tempo ativo e intervalo MUST continuar acessíveis, e o cartão MUST manter foco e seleção por teclado e ponteiro.

#### Scenario: Cartão curto com título longo
- **WHEN** uma frente curta não tem altura para exibir título, metadados e notas juntos
- **THEN** o cartão preserva um título identificável sem texto sobreposto, oculta apenas conteúdo que não cabe e mantém os dados completos no nome acessível e no fluxo de seleção existente
- Test: integration — estados compacto/intermediário/alto em pt-BR e EN, com título longo e foco/seleção.

#### Scenario: Cartão maior com espaço para informações
- **WHEN** a altura ou largura disponível permite mostrar mais conteúdo
- **THEN** título, tempo ativo, intervalo, retomadas e notas são revelados em ordem de prioridade, respeitando a tipografia e os tokens do Clovy
- Test: integration — smoke visual com os mesmos dados em alturas e larguras distintas.

### Requirement: Colunas coerentes com o retângulo desenhado
A distribuição em colunas SHALL considerar a escala adaptada, a altura mínima real do cartão e a separação visual. Todos os cartões MUST permanecer legíveis e selecionáveis, sem que um retângulo cubra outro.

#### Scenario: Tarefas sequenciais curtas que cabem na mesma coluna
- **WHEN** duas tarefas de 10 minutos começam às 09:00 e às 09:30 num calendário de 600 pixels úteis
- **THEN** elas compartilham a mesma coluna, conservam seus horários e não se encobrem
- Test: integration — testemunha RED/GREEN de geometria e cartões renderizados.

#### Scenario: Sobreposição real e retomadas
- **WHEN** duas frentes têm intervalos sobrepostos ou uma frente reaparece em períodos separados por pausas de até 45 minutos
- **THEN** conflitos visuais são distribuídos em colunas e a frente retomada continua em um cartão, com os períodos como barras separadas por vãos e duração ativa independente de seu intervalo total
- Test: integration — regressão de sobreposição, pausa e retomada no calendário.

#### Scenario: Frente retomada depois de uma pausa longa
- **WHEN** o intervalo entre o fim de um período medido de uma frente e o início do próximo passa de 45 minutos
- **THEN** o trabalho seguinte começa um novo cartão da mesma frente, no horário real; cada cartão mostra o título da frente, o intervalo e os períodos do seu trecho e os minutos ativos medidos nesse trecho (os minutos localizados de uma hora vão para os trechos dos seus períodos, em proporção a eles), e a soma dos cartões é o total da frente
- **AND** os minutos de uma hora que nenhum período localizou ficam no cartão da parte aproximada dessa hora, com barra tracejada e selo "Por hora"
- **AND** selecionar qualquer cartão seleciona a frente inteira (todos os seus cartões em destaque e o detalhe da frente no painel lateral), e o cabeçalho continua contando frentes e minutos uma vez só
- Test: integration — `workstream-timeline-split.test.tsx` e `workstream-timeline-minutes.test.tsx`, com captura do dia de 12 frentes espalhadas.

### Requirement: Preservação das superfícies fora do calendário
A mudança MUST preservar o painel lateral, o agrupamento e a geração das frentes, os dados, os provedores, a busca, a navegação e o estado de dias sem frentes. Dados legados MUST continuar marcados como aproximados, sem inventar precisão temporal.

#### Scenario: Seleção, navegação, legado e estado vazio
- **WHEN** o usuário seleciona uma frente, muda de dia, busca uma sessão ou abre um dia vazio ou com dados por hora
- **THEN** os fluxos existentes continuam funcionando, a lateral mantém seu conteúdo/comportamento e a grade não inventa tarefas nem horários precisos para registros aproximados
- Test: integration — controles existentes de seleção/navegação/legado/sem frentes e conferência do escopo do diff.
