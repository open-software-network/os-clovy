# clovy-mcp-server Specification

## Purpose
Expõe os dados do Clovy (notas, transcrições, ditados, memórias e atividade) a clientes MCP locais como Claude Code e Cursor, e fornece as ferramentas do Clovy aos motores CLI do chat.

## Requirements

### Requirement: Servidor MCP local opcional
O Clovy SHALL oferecer um servidor MCP por stdio, desligado por padrão, que só responde enquanto o Clovy está aberto e o servidor está ligado nas configurações. A conexão entre o processo stdio e o app MUST ser local, restrita ao usuário e autenticada por um segredo da instalação. Com o app fechado ou o servidor desligado, as ferramentas MUST responder com erro claro.

#### Scenario: Servidor desligado
- **WHEN** um cliente MCP chama uma ferramenta com o servidor desligado
- **THEN** recebe erro informando que o servidor MCP do Clovy está desligado
- Test: integration — processo stdio contra app de teste com servidor desligado

#### Scenario: Conexão de outro usuário
- **WHEN** um processo sem o segredo da instalação tenta conectar ao canal local
- **THEN** a conexão é recusada
- Test: integration — conexão sem segredo

### Requirement: Ferramentas de dados do Clovy
O servidor SHALL expor, somente leitura, ferramentas com nomes `verb_object`: `search_notes`, `get_note`, `list_dictations`, `list_memories`, e as de atividade `get_activity_timeline`, `search_activity`, `get_activity_stats`, `get_active_session`, `list_app_usage`, `get_session_detail`, `list_coding_agent_sessions`, `get_day_summary`, `get_day_panels`, `get_day_schedule`. Notas, ditados e memórias MUST vir do perfil ativo. A atividade (captura, linha do tempo, blocos de agentes, resumos) é da instalação, sem partição por perfil, como na vista "Hoje" e no agente do app. Ferramentas de atividade MUST existir apenas com a captura ligada e respeitar exclusões, retenção e limites de saída.

#### Scenario: Busca de notas pelo Claude Code
- **WHEN** o Claude Code, configurado com o servidor do Clovy, chama `search_notes` com um termo de uma nota existente
- **THEN** recebe a nota com título e trecho, do perfil ativo
- Test: integration — chamada MCP real ao servidor com banco de teste

#### Scenario: Painéis e agenda do resumo do dia
- **WHEN** um cliente MCP chama `get_day_panels` para um dia e `get_day_schedule`
- **THEN** recebe os números que o Resumo do dia desenha (foco, ocioso, ausente, categorias, apps, foco por hora, minutos por frente, reuniões da partição aberta, blocos de agentes de código das fontes ligadas, rateados no dia), sem apps e domínios excluídos nem nada antes da retenção, e o horário do resumo com as notificações e o horário de silêncio
- Test: integration — `mcp_server::tests::day_panels_draw_the_day_from_filtered_activity_meetings_and_coding_agents`, `mcp_server::tests::day_schedule_reports_the_summary_time_and_notification_settings`

### Requirement: Recursos de contexto
O servidor SHALL expor os recursos `clovy://context` (data, hora e fuso atuais) e `clovy://guide` (guia de uso das ferramentas para modelos).

#### Scenario: Contexto atual
- **WHEN** um cliente lê `clovy://context`
- **THEN** recebe data, hora e fuso do sistema
- Test: unit — handler de recursos

### Requirement: Configuração para clientes
As configurações SHALL mostrar o comando e o trecho de configuração prontos para Claude Code e Cursor, com botão de copiar, e o servidor MUST poder ser oferecido automaticamente às sessões de chat com motor CLI que aceitam MCP.

#### Scenario: Copiar configuração
- **WHEN** o usuário liga o servidor e clica em copiar a configuração do Claude Code
- **THEN** o clipboard recebe um trecho válido que aponta para o executável do servidor do Clovy instalado
- Test: unit — gerador do trecho de configuração

### Requirement: Vínculo de turno
Cada turno CLI com ferramentas do Clovy SHALL receber um token de turno de uso único nos argumentos do `clovy-mcp` (`--turn <token>`), registrado pelo app com sessão, execução, CLI, modo de segurança e se o turno é assistido. O relay MUST enviar o token no handshake como campo aditivo `turn`, mantendo `clovyMcp: 1`. O token MUST NOT dispensar o segredo da instalação nem a checagem de usuário.

#### Scenario: Turno vinculado reconhecido
- **WHEN** um turno Claude ou Codex inicia com o servidor ligado
- **THEN** a conexão do `clovy-mcp` desse turno é associada à sessão e à execução corretas e lista as ferramentas de escrita
- Test: integration — CLI falso que conecta pelo relay real com `--turn`, `tools/list` e registro do app

### Requirement: Revogação do token de turno
O app MUST revogar o token quando a execução termina, falha ou é cancelada e MUST fechar as conexões vinculadas a ele. Token desconhecido ou revogado MUST ser recusado com `turn_expired`.

#### Scenario: Token revogado no fim da execução
- **WHEN** a execução é cancelada e um processo reusa o mesmo token
- **THEN** o handshake é recusado com `turn_expired` e nenhuma ferramenta executa
- Test: integration — socket real, cancelamento pelo `ChatEngineHost` e nova conexão com o token antigo

### Requirement: Clientes sem vínculo continuam só leitura
Conexões sem token de turno SHALL ver exatamente as ferramentas e recursos de leitura atuais. Ferramentas de escrita MUST NOT aparecer em `tools/list` sem vínculo, e chamá-las MUST retornar `isError` com `turn_required`.

#### Scenario: Cliente externo
- **WHEN** um cliente configurado pelo snippet de Configurações chama `tools/list` e depois `save_memory`
- **THEN** a lista é a de leitura e a chamada falha com `turn_required`, sem gravar memória
- Test: integration — relay real sem `--turn`, comparação com o catálogo de leitura e banco inalterado

### Requirement: Catálogo de paridade com nomes fixos
O servidor SHALL oferecer a turnos vinculados as ferramentas dos três requisitos seguintes, com os mesmos argumentos, limites, gates e erros das ferramentas nativas equivalentes, salvo onde indicado. Nomes MUST ser exatamente os das tabelas; este delta é o dono deles.

#### Scenario: Paridade de leitura paginada
- **WHEN** um turno vinculado lê uma nota com conteúdo maior que uma página e um ditado longo por id
- **THEN** recebe páginas sucessivas com `nextContentOffset` e `nextTextOffset` até o fim, sem perder caracteres
- Test: unit — banco de teste com textos multibyte longos, comparando a concatenação com o original

### Requirement: Ferramentas de leitura e mídia sem broker
Estas ferramentas SHALL executar sem aprovação do broker, como no motor nativo.

| Grupo | Ferramentas |
|---|---|
| Notas | `get_dictation` (`id`, `textOffset`; paginado; respeita a retenção de ditados do MCP); `get_note` ganha `contentOffset` |
| Mídia | `generate_image`, `edit_image`, `generate_video` (artefato no workspace da sessão e em `agent_artifacts`) |
| Gravador | `recording_status` |

#### Scenario: Mídia exige vínculo
- **WHEN** um turno vinculado chama `generate_image`
- **THEN** o arquivo aparece em `artifacts/` do workspace da sessão e na lista de artefatos da sessão
- Test: integration — `media_generate_image_reaches_fal_directly_over_http` usa fal falsa; `media_generate_image_lands_in_the_session_workspace_and_artifact_list` verifica o arquivo e a lista de artefatos da sessão

### Requirement: Leituras de vault, skills e rotinas
`get_obsidian_vault`, `list_skills`, `load_skill` e `list_routines` SHALL executar sem aprovação do broker; skills MUST respeitar a habilitação global de `agent_skill_settings`.

#### Scenario: Skill desabilitada
- **WHEN** um turno vinculado chama `load_skill` com uma skill desabilitada
- **THEN** a ferramenta recusa e `list_skills` não a lista
- Test: unit — diretório de skills de teste e habilitação gravada

### Requirement: Leituras de conectores
Estas leituras SHALL executar sem aprovação do broker, com o escopo e a revalidação de conta atuais.

| Grupo | Ferramentas |
|---|---|
| Gmail, Calendar | `search_threads`, `read_thread`, `list_unread`, `list_events`, `get_event`, `find_free_slots` |
| GitHub | `list_repositories`, `get_pull_request`, `search_issues`, `get_issue`, `list_issue_comments` |
| Notion | leitura `notion-*` da allowlist atual |

#### Scenario: Leitura de conector sem cartão
- **WHEN** um turno vinculado chama `list_unread` de uma conta conectada
- **THEN** recebe as conversas sem cartão de aprovação e com o escopo revalidado
- Test: integration — servidor HTTP falso do Google e conta de teste

### Requirement: Ferramentas de escrita com broker
Estas ferramentas MUST obter aprovação pelo broker antes de qualquer efeito.

| Grupo | Ferramentas |
|---|---|
| Memória | `save_memory`, `forget_memory` |
| Gravador | `start_recording`, `stop_recording` |
| Computer use | `computer_use` (a cada chamada) |
| Rotinas | `create_routine`, `update_routine`, `pause_routine`, `resume_routine`, `delete_routine` |
| Interação | `request_clarification` (tipo clarificação); `approve_action` (só turno Claude, para `--permission-prompt-tool`) |

#### Scenario: Escrita aprovada
- **WHEN** um turno vinculado chama `save_memory` e o usuário aprova o cartão
- **THEN** a memória é gravada uma vez e a ferramenta devolve o resultado
- Test: integration — broker real com resposta pelo comando de resolução e leitura do banco

### Requirement: Mutações de conectores com broker
Estas mutações MUST obter aprovação pelo broker antes de qualquer chamada externa.

| Conector | Ferramentas |
|---|---|
| Gmail | `create_draft`, `send_email`, `modify_labels`, `archive` |
| Calendar | `create_event`, `respond_to_invite` |
| GitHub | `create_issue`, `update_issue`, `add_comment` |
| Notion | `notion-create-pages`, `notion-update-page` |

#### Scenario: Escrita negada
- **WHEN** um turno vinculado chama `send_email` e o usuário nega
- **THEN** a ferramenta devolve `isError` com `user_denied` e nenhuma chamada ao Google é feita
- Test: integration — servidor HTTP falso do Google sem requisições registradas

### Requirement: Browser use com a aprovação do próprio broker do navegador
As 14 ferramentas `start_session`, `close_session`, `navigate`, `snapshot`, `screenshot`, `click`, `fill`, `press`, `back`, `list_tabs`, `open_tab`, `switch_tab`, `close_tab`, `accept_shared_tab` SHALL operar o `BrowserBroker` no contexto assistido do vínculo; ações consequentes MUST usar a fila de aprovação existente do navegador, sem pedido duplicado ao broker de aprovação.

#### Scenario: Clique consequente
- **WHEN** um turno vinculado clica num botão de envio em origem ainda não permitida
- **THEN** a ação fica na bandeja de aprovações do navegador e só executa após a resposta
- Test: integration — transporte falso do broker do navegador

### Requirement: Gateway de MCP do usuário e do Linear
Para turnos vinculados, o servidor SHALL listar as ferramentas habilitadas dos servidores MCP do usuário e do Linear gerenciado com os nomes `mcp_<servidor>_<tool>` e `mcp_linear_*` e executá-las pelo caminho existente do app (visibilidade, timeout, limite de saída, Seatbelt em sessão `sandboxed`). Ferramentas que a política do servidor marca como exigindo aprovação MUST passar pelo broker.

#### Scenario: Ferramenta do usuário com aprovação
- **WHEN** um turno chama uma ferramenta de servidor do usuário com aprovação padrão
- **THEN** o cartão aparece, e só depois de aprovado o servidor recebe a chamada com o header do Keychain
- Test: integration — servidor MCP HTTP falso que registra headers e chamadas

### Requirement: Credenciais de MCP ficam no app
Credenciais, headers e tokens de servidores MCP e conectores MUST permanecer no Keychain e na memória do app e MUST NOT ser escritos em argv, ambiente ou arquivo lido pelo CLI.

#### Scenario: Linear sem token no CLI
- **WHEN** um turno Codex usa `mcp_linear_*`
- **THEN** o argv, o ambiente e os arquivos temporários do CLI não contêm o token do Linear
- Test: integration — CLI falso que grava argv/env/arquivos e busca pelo token de teste
