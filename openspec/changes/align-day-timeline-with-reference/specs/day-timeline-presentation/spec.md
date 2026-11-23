# Spec Delta

## Purpose

Mostrar o dia em uma grade temporal de frentes de trabalho, permitindo entender atividades e retomadas sem confundir tempo decorrido com tempo efetivamente trabalhado.

## ADDED Requirements

### Requirement: Grade temporal por frentes de trabalho

A vista Hoje SHALL apresentar horas num eixo vertical, cartões de frentes de trabalho e os períodos associados a cada frente. A interface MUST usar os tokens e o idioma do Clovy, não reproduzir a paleta do Meridian.

#### Scenario: Trabalho retomado em horários diferentes
- **WHEN** a mesma frente tem atividade em períodos separados por outras atividades ou pausas
- **THEN** a grade mostra um cartão agrupado, marcadores dos períodos, duração efetiva sem somar o intervalo de pausa e posicionamento temporal coerente
- Test: integration — regressão comportamental de retomadas e smoke visual em dev com dois períodos separados.

#### Scenario: Frentes sobrepostas
- **WHEN** frentes ocupam intervalos que se sobrepõem no eixo temporal
- **THEN** os cartões ficam em colunas que permitem ler e selecionar cada frente, sem ocultar outro cartão
- Test: integration — regressão de layout com sobreposição e smoke visual em dev.

#### Scenario: Dados históricos sem precisão temporal
- **WHEN** um registro legado só contém minutos agregados por hora
- **THEN** a interface indica essa granularidade e não apresenta minutos de início/fim inventados como se fossem medidos
- Test: unit — regressão de apresentação de registro legado.

#### Scenario: Dia ainda sem frentes geradas
- **WHEN** há captura mas nenhuma frente foi gerada ou não há provedor de atividade configurado
- **THEN** a vista preserva acesso às sessões capturadas, comunica o estado sem criar tarefas falsas e permite acompanhar o dia
- Test: integration — regressão de estado sem frentes e smoke em dev.

### Requirement: Painel lateral de panorama e detalhes

A grade SHALL exibir panorama do dia sem seleção e detalhes da frente selecionada sem tirar a grade da tela.

#### Scenario: Selecionar uma frente
- **WHEN** o usuário seleciona um cartão
- **THEN** o painel mostra o título, tempo efetivo, períodos/retomadas disponíveis e o que foi feito, destaca o cartão e permite voltar ao panorama
- Test: integration — regressão de seleção e smoke visual selecionando e fechando uma frente.

#### Scenario: Navegação e atualização
- **WHEN** o usuário muda de dia, busca uma sessão ou recebe atualização de captura/inteligência
- **THEN** a vista apresenta dados do dia correto, mantém busca e acesso ao resumo e não conserva seleção de outro dia
- Test: integration — regressão de navegação e smoke em dev.

### Requirement: Ícones reais dos principais aplicativos

Principais apps SHALL mostrar o ícone real do aplicativo instalado no macOS, resolvido pela identidade do app, sem requisição externa para obter imagens.

#### Scenario: Aplicativos instalados com ícones distintos
- **WHEN** Principais apps lista dois aplicativos instalados com identidades diferentes
- **THEN** cada linha mostra o ícone real correspondente, seu nome e tempo de uso, sem usar um ícone genérico no lugar de um ícone disponível
- Test: e2e — smoke nativo em dev com dois apps instalados e observação dos ícones locais.

#### Scenario: Aplicativo ausente ou sem ícone acessível
- **WHEN** um aplicativo histórico foi removido ou seu ícone não pode ser lido
- **THEN** a linha preserva nome/tempo com fallback neutro, sem usar o ícone de outro app nem quebrar a lista
- Test: integration — regressão comportamental da resolução indisponível.
