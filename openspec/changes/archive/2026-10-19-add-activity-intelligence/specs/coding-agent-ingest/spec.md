# Spec Delta

## Purpose

Traz para o Clovy o trabalho feito em agentes de código locais (o que a captura de tela não vê), lendo suas transcrições, segmentando em blocos e resumindo cada bloco.

## ADDED Requirements

### Requirement: Fontes suportadas e somente leitura
O sistema SHALL ler, quando a fonte estiver ligada, as transcrições locais de Claude Code, Codex CLI, Copilot CLI, Copilot no VS Code, Cursor, Cursor CLI e Antigravity, a partir de seus locais padrão. Cada fonte MUST poder ser ligada ou desligada nas configurações, e os arquivos das fontes MUST ser apenas lidos, nunca alterados.

#### Scenario: Fonte desligada
- **WHEN** a fonte Codex está desligada
- **THEN** nenhum arquivo em `~/.codex/sessions` é lido
- Test: unit — varredura com fonte desligada e diretório de teste

#### Scenario: Arquivos intactos
- **WHEN** a ingestão processa uma transcrição do Claude Code
- **THEN** o arquivo de origem mantém conteúdo e data de modificação
- Test: integration — ingestão sobre cópia e comparação de hash/mtime

### Requirement: Segmentação em blocos
O sistema SHALL normalizar cada transcrição (horários, diretório de trabalho, prompts humanos distintos de resultados de ferramentas) e cortar blocos em pausas acima de 1 hora ou a cada 1 hora, sempre em fronteira de prompt do usuário. Blocos em andamento MUST ser atualizados até serem selados.

#### Scenario: Conversa longa
- **WHEN** uma conversa dura 2 h 30 min sem pausas
- **THEN** ela vira três blocos cortados em prompts do usuário
- Test: unit — segmentador com transcrição sintética

### Requirement: Resumo de blocos selados
Blocos selados SHALL ser resumidos pelo CLI do próprio agente quando ele estiver instalado e puder rodar com todas as ferramentas desligadas e, caso contrário, pelo provedor de atividade escolhido. A transcrição MUST ir ao modelo como dado citado e não confiável. Sem nenhum dos dois, o bloco MUST ficar selado sem resumo e nenhum conteúdo MUST sair da máquina.

#### Scenario: Sem provedor
- **WHEN** um bloco do Cursor é selado, o `cursor-agent` não está instalado e não há provedor de atividade
- **THEN** o bloco permanece selado sem resumo e nenhuma chamada é feita
- Test: unit — seletor de resumidor sem CLI e sem provedor

### Requirement: Proteção contra autoingestão
Conversas criadas pelas próprias chamadas do Clovy a CLIs (marcadas com o marcador de autoria do Clovy) MUST ser ignoradas pela ingestão.

#### Scenario: Chamada do Clovy ignorada
- **WHEN** o Clovy resume um bloco usando `claude` e o Claude Code grava essa conversa
- **THEN** a ingestão não cria bloco para essa conversa
- Test: unit — filtro por marcador sobre transcrição sintética

### Requirement: Sessões de agentes na linha do tempo
Blocos de agentes de código SHALL aparecer na vista "Hoje" como uma faixa própria com agente, projeto, horário e resumo, e SHALL alimentar os relatórios por hora e o resumo do dia.

#### Scenario: Bloco visível no dia
- **WHEN** existe um bloco resumido do Codex entre 10:00 e 10:40
- **THEN** a vista "Hoje" mostra o bloco nesse intervalo com o resumo
- Test: e2e — app de desenvolvimento com bloco de teste, captura de tela da vista
