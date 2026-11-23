# Spec Delta

## Purpose

Permite que o usuário use seus próprios provedores de IA no Clovy (CLIs de agentes locais e endpoints OpenAI-compatíveis) para geração pontual e como motor do chat, sem depender do Clovy API.

## ADDED Requirements

### Requirement: Catálogo de provedores CLI detectados
O sistema SHALL reconhecer os CLIs `claude`, `codex`, `pi`, `agy`, `cursor-agent` e `copilot`, resolvendo-os pelo PATH do shell de login do usuário (não apenas pelo PATH herdado pelo app gráfico) e exibindo, para cada um, se está instalado, o caminho resolvido e a versão quando o CLI a informar. O ambiente do perfil do usuário (por exemplo `HOME` real e variáveis como `PI_CODING_AGENT_DIR`) MUST ser preservado nas execuções.

#### Scenario: CLI instalado fora do PATH do app gráfico
- **WHEN** o `claude` existe apenas em um diretório adicionado pelo shell de login (por exemplo `~/.local/bin`) e o usuário abre a lista de provedores
- **THEN** o Clovy mostra `claude` como instalado com o caminho resolvido e permite selecioná-lo
- Test: unit — resolução de PATH via shell de login com executável falso em diretório temporário

#### Scenario: CLI ausente
- **WHEN** nenhum executável `cursor-agent` é encontrado
- **THEN** o provedor aparece como não instalado, não pode ser selecionado e o motivo é exibido
- Test: unit — detecção com PATH sem o executável

### Requirement: Endpoints OpenAI-compatíveis nomeados
O sistema SHALL permitir cadastrar vários endpoints OpenAI-compatíveis, cada um com nome, URL base, modelo e chave opcional. A chave MUST ficar no Keychain e nunca ser devolvida à interface nem registrada em log. O endpoint local já existente nas configurações do Clovy MUST ser migrado como um endpoint nomeado, sem perda de configuração.

#### Scenario: Migração do endpoint local existente
- **WHEN** o Clovy inicia com `provider-settings.json` contendo `localGeneration` preenchido
- **THEN** a lista de provedores contém um endpoint com a mesma URL, modelo e chave, e as rotas que usavam o endpoint local continuam usando-o
- Test: unit — carga de settings legado e verificação do registro migrado

#### Scenario: Chave não volta à interface
- **WHEN** a interface lista os endpoints cadastrados
- **THEN** cada endpoint informa apenas se possui chave, sem o valor
- Test: unit — DTO serializado sem o campo de segredo

### Requirement: Teste de conexão e capacidade de saída estruturada
O sistema SHALL oferecer, para cada provedor, um teste de conexão que executa um prompt mínimo e mostra sucesso com latência ou o erro recebido, e SHALL registrar o nível de saída estruturada suportado (`none`, `prompt`, `json_object`, `json_schema`, `strict`). Recursos que exigem JSON estruturado MUST recusar provedores abaixo de `json_schema` com mensagem explicativa.

#### Scenario: Teste de conexão bem-sucedido
- **WHEN** o usuário aciona "Testar" em um provedor configurado e acessível
- **THEN** a interface mostra sucesso com a latência medida e o nível de saída estruturada detectado
- Test: integration — CLI falso e servidor HTTP falso respondendo ao prompt de teste

#### Scenario: Provedor sem saída estruturada suficiente
- **WHEN** o usuário escolhe para os recursos de atividade um endpoint cujo nível detectado é `prompt`
- **THEN** a escolha é recusada e a interface explica que é necessário suporte a JSON schema
- Test: unit — validação de seleção por nível de capacidade

### Requirement: Seleção de provedor por uso
O sistema SHALL permitir escolher o provedor separadamente para: chat/agente, geração de notas, limpeza de ditado e recursos de atividade (relatórios, resumo do dia, resumo de sessões de agentes). Clovy API MUST continuar disponível para chat, notas e ditado. Para recursos de atividade, o padrão MUST ser "nenhum", e sem provedor escolhido esses recursos ficam desligados, sem fallback para o Clovy API.

#### Scenario: Atividade sem provedor
- **WHEN** nenhum provedor de atividade foi escolhido e chega a hora de gerar o resumo do dia
- **THEN** nenhum texto de atividade sai da máquina e a interface mostra que o recurso precisa de um provedor configurado
- Test: integration — agendador com provedor "nenhum" não realiza chamada e registra estado desligado

#### Scenario: Nota gerada por CLI
- **WHEN** o provedor de notas é `claude` e uma gravação termina de ser transcrita
- **THEN** a nota estruturada é gerada pelo CLI e aparece no editor como nas notas geradas pelo Clovy API
- Test: integration — CLI falso devolvendo nota e verificação da nota persistida

### Requirement: Execução isolada de CLIs em geração pontual
Chamadas de geração pontual a CLIs SHALL rodar sem persistir sessão no CLI, sem ferramentas e sem carregar arquivos de contexto/skills, com tempo limite, e MUST marcar o prompt com o marcador de autoria do Clovy para que a ingestão de sessões de agentes as ignore. CLIs que não garantem todas as ferramentas desligadas (hoje `codex`, `agy`, `cursor-agent`, `copilot`) MUST ser recusados para notas, limpeza de ditado e atividade, sem redirecionamento silencioso; continuam disponíveis como motor do chat. Chamadas em segundo plano dos recursos de atividade MUST ser serializadas (uma por vez).

#### Scenario: CLI que mantém ferramentas
- **WHEN** o usuário tenta escolher `codex` para notas
- **THEN** a escolha é recusada com a explicação de que as ferramentas desse CLI não podem ser desligadas, e nenhum processo do `codex` é iniciado
- Test: unit — seleção por uso e invocação recusadas com CLI falso instalado

#### Scenario: Marcador de autoria presente
- **WHEN** o Clovy executa qualquer chamada de geração pontual por CLI
- **THEN** o prompt enviado contém o marcador de autoria do Clovy
- Test: unit — montagem do comando/prompt por provedor

#### Scenario: Tempo limite
- **WHEN** um CLI não responde dentro do tempo limite configurado
- **THEN** o processo é encerrado e a chamada falha com erro de tempo esgotado, sem processo órfão
- Test: integration — CLI falso que dorme além do limite

### Requirement: CLI como motor do chat
O sistema SHALL permitir escolher um CLI como motor de uma sessão de chat. Nessa sessão, cada mensagem do usuário MUST ser entregue ao CLI com continuidade da conversa anterior, e texto, raciocínio e atividade de ferramentas emitidos pelo CLI MUST aparecer na interface de sessão do Clovy e ser persistidos como itens da sessão. Cancelar MUST encerrar o processo do CLI. Quando o CLI aceitar servidores MCP, as ferramentas do Clovy MUST ser oferecidas pelo servidor MCP do Clovy; quando não aceitar, a interface MUST indicar que a sessão não tem ferramentas do Clovy. Mensagens enviadas durante uma execução MUST virar mensagem seguinte em vez de serem perdidas.

#### Scenario: Conversa com continuidade
- **WHEN** o usuário envia duas mensagens seguidas numa sessão com motor `claude`
- **THEN** a segunda execução retoma a mesma conversa do CLI e ambas as respostas aparecem em ordem na sessão, persistidas após reiniciar o app
- Test: integration — CLI falso que exige o identificador de retomada na segunda chamada

#### Scenario: Cancelamento
- **WHEN** o usuário cancela uma execução em andamento com motor CLI
- **THEN** o processo do CLI termina e a execução fica marcada como cancelada
- Test: integration — CLI falso de longa duração e verificação de término do processo

### Requirement: Endpoint OpenAI-compatível como modelo do agente
O sistema SHALL permitir usar qualquer endpoint cadastrado como modelo do agente do Clovy, mantendo as ferramentas do Clovy, streaming e direcionamento durante a execução.

#### Scenario: Agente com endpoint próprio
- **WHEN** o usuário escolhe um endpoint cadastrado como modelo do chat e pede uma tarefa que usa `search_june`
- **THEN** a resposta chega em streaming pelo endpoint e a chamada de ferramenta aparece na sessão
- Test: integration — servidor OpenAI-compatível falso com resposta de tool call
