# activity-timeline Specification

## Purpose
Transforma a atividade capturada numa linha do tempo do dia com sessões, lacunas, categorias, estatísticas e busca, visível na interface do Clovy e consultável pelo agente.

## Requirements

### Requirement: Sessões por app
O sistema SHALL agrupar quadros consecutivos do mesmo app em sessões, distinguindo domínio do navegador e projeto/workspace do editor como contexto da sessão, e SHALL manter uma sessão ativa em andamento. Trocar o app em foco MUST fechar a sessão anterior e abrir uma nova. O processamento MUST ser incremental e retomar do último quadro processado após reinício.

#### Scenario: Troca de app
- **WHEN** o usuário usa o editor por 10 minutos e depois o navegador por 5 minutos
- **THEN** a linha do tempo mostra uma sessão fechada do editor de ~10 minutos e uma sessão ativa do navegador
- Test: unit — construtor de sessões com sequência de quadros sintéticos

#### Scenario: Retomada após reinício
- **WHEN** o Clovy reinicia no meio do processamento
- **THEN** nenhum quadro é processado duas vezes e nenhum é pulado
- Test: integration — processamento interrompido e retomado sobre banco de teste

### Requirement: Lacunas e tempo real
Intervalos acima de 5 minutos sem quadros úteis SHALL virar lacunas classificadas como ocioso, suspensão do sistema ou pausa (manual, horário de trabalho, pouco disco, vídeo protegido). Uma lacuna ainda em andamento MUST aparecer até o momento atual. A duração das sessões MUST excluir lacunas, de modo que suspensão ou noite nunca inflem as horas registradas.

#### Scenario: Mac suspenso
- **WHEN** não há quadros entre 12:00 e 13:10 porque o Mac dormiu
- **THEN** a linha do tempo mostra uma lacuna "suspensão" nesse intervalo e o tempo total do dia não a inclui
- Test: unit — classificação de lacunas com quadros sintéticos

### Requirement: Categorização determinística
Cada sessão fechada SHALL receber uma categoria entre: programação, revisão de código, reunião, comunicação, design, documentação, planejamento, deploy/devops, pesquisa, ocioso/pessoal, por pontuação de evidências (app, título, texto capturado e presença de áudio de reunião), sem chamar modelo. Quando a confiança ficar abaixo do piso, a categoria MUST ser ocioso/pessoal.

#### Scenario: Sessão de reunião
- **WHEN** a sessão é do Zoom com áudio de reunião ativo
- **THEN** a categoria é reunião
- Test: unit — categorizador com amostras por categoria

### Requirement: Vista "Hoje" com linha do tempo
O Clovy SHALL ter uma vista de primeiro nível na barra lateral com a linha do tempo do dia: navegação entre dias, blocos por sessão coloridos por categoria, lacunas, sessão ativa ao vivo e detalhe da sessão (janelas, URL, trecho de texto). A vista MUST seguir as regras de UI do Clovy (`spec/`) e existir em inglês e português do Brasil.

#### Scenario: Abrir o dia anterior
- **WHEN** o usuário abre a vista e navega para ontem
- **THEN** vê as sessões e lacunas de ontem e, ao clicar numa sessão, o detalhe com janelas e trecho de texto
- Test: e2e — app de desenvolvimento com dados de atividade de teste, captura de tela da vista

#### Scenario: Captura desligada
- **WHEN** a captura nunca foi ligada
- **THEN** a vista explica o recurso e oferece ligá-lo, sem erro
- Test: unit — componente da vista em estado vazio

### Requirement: Estatísticas do dia
A vista SHALL mostrar tempo focado, ocioso e ausente, os apps mais usados e a distribuição por categoria do dia selecionado, calculados a partir das sessões e lacunas gravadas.

#### Scenario: Totais coerentes
- **WHEN** o dia tem 6 h de sessões e 1 h de lacunas ociosas
- **THEN** as estatísticas mostram 6 h focadas e 1 h ociosa, e a soma por categoria é 6 h
- Test: unit — agregador de estatísticas

### Requirement: Busca no histórico de atividade
O usuário SHALL poder buscar texto em títulos de janela, URLs e texto capturado, com filtro de período, e abrir o resultado na linha do tempo no momento correspondente. A busca MUST respeitar o período de retenção.

#### Scenario: Encontrar uma página
- **WHEN** o usuário busca uma palavra que apareceu numa página três dias atrás
- **THEN** o resultado mostra a sessão e o horário, e ao clicar a linha do tempo abre nesse ponto
- Test: integration — índice de busca sobre banco de teste

### Requirement: Ferramentas de atividade para o agente
O agente do Clovy SHALL ter as ferramentas `search_activity` (busca no histórico) e `get_activity_timeline` (sessões e lacunas de um período), disponíveis apenas quando a captura estiver ligada, e MUST respeitar as mesmas exclusões e retenção da captura.

#### Scenario: Pergunta sobre o dia
- **WHEN** o usuário pergunta ao agente "o que eu fiz hoje de manhã?" com a captura ligada
- **THEN** o agente chama `get_activity_timeline` para o período e responde com base nas sessões
- Test: integration — despacho da ferramenta sobre banco de teste

### Requirement: Quadro útil
Um quadro SHALL ser útil quando o app não é de tela bloqueada ou protetor de tela e ao menos um destes vale: a janela (app, título ou URL) mudou, o texto mudou, houve evento de entrada gravado perto do quadro, ou o tempo desde a última entrada do sistema é menor ou igual ao intervalo entre ticks mais 1 segundo. Os demais quadros SHALL ser ociosos. Sem o tempo do sistema no quadro, vale só o restante da regra.

#### Scenario: Leitura com rolagem
- **WHEN** o usuário passa 10 minutos lendo a mesma página, sem clique nem tecla, rolando a cada poucos segundos
- **THEN** os quadros são úteis, o intervalo conta como sessão focada e não aparece lacuna ociosa
- Test: unit — construtor com quadros iguais e tempo desde a última entrada de 1 s

#### Scenario: Tela parada sem entrada
- **WHEN** a janela não muda por 6 minutos e o tempo desde a última entrada cresce em todos os quadros
- **THEN** os quadros são ociosos e o intervalo vira lacuna ociosa
- Test: unit — construtor com quadros iguais e tempo desde a última entrada crescente

#### Scenario: Quadros antigos sem o dado
- **WHEN** a linha do tempo é reconstruída com quadros gravados antes desta mudança
- **THEN** a classificação desses quadros é a mesma de antes
- Test: unit — construtor com quadros sem tempo desde a última entrada

### Requirement: Tela bloqueada no Windows
Quadros cujo app é a tela de bloqueio ou de login do Windows (`LockApp`, `LogonUI`) SHALL nunca ser úteis, mesmo com entrada do usuário. O intervalo de bloqueio MUST ser contabilizado como "Ausente", não como "Focado" ou "Ocioso", inclusive enquanto o bloqueio está em andamento. A classificação preexistente do macOS MUST permanecer inalterada.

#### Scenario: Windows bloqueado com o Clovy aberto
- **WHEN** o usuário bloqueia o Windows por 20 minutos com a captura ligada
- **THEN** o intervalo conta como "Ausente", não como tempo focado ou ocioso, tanto antes quanto depois do desbloqueio
- Test: unit — construtor com quadros de `LockApp` e `LogonUI` com entrada recente, preservando os resultados macOS; e2e — sessão Windows bloqueada e linha do tempo lida pelo app real
