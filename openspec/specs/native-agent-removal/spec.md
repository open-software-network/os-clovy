# native-agent-removal Specification

## Purpose
Define quando e como o agente nativo é apagado, o que sobrevive e como os usuários existentes atravessam o corte.

## Requirements

### Requirement: Critério de corte
A entrega do corte SHALL exigir fases 0 e 2 concluídas, `make verify` sem falhas novas, contratos locais de Claude e Codex cobertos por testes, nenhum ponto de entrada iniciando `run.start`, smoke reduzido das tarefas 3.2 e 3.3 e aceite explícito do dono. As duas falhas de formatação preexistentes MUST ser registradas separadamente. A implementação pode anteceder o smoke, conforme decisão do orquestrador de 2026-10-28.

#### Scenario: Corte bloqueado
- **WHEN** algum ponto de entrada ainda inicia o runtime nativo
- **THEN** a entrega da fase 4 fica bloqueada até remover esse ponto de entrada
- Test: static — busca por `run.start` e `AgentRuntimeHost` no código vigente retorna vazio; integration — evidência reduzida e aceite registrados nas tarefas 3.2 e 3.3

#### Scenario: Smoke reduzido aceito após o corte
- **WHEN** o dono aceita o corte e o orquestrador registra a redução em 2026-10-28
- **THEN** a evidência Claude pós-corte com MCP, perfil sintético novo e identidade própria, junto ao relatório da tentativa pré-corte, satisfaz o smoke reduzido; paridade nativa pré-corte, perfil copiado e turno Codex real não são exigidos
- **AND** Codex real permanece explicitamente não comprovado
- Test: manual — evidência e decisão registradas nas tarefas 3.2 e 3.3, sem reclassificar testes com CLI falso como prova nativa

### Requirement: Nada do sidecar no produto
Depois do corte, o bundle, os scripts de build, o CI e o processo do app MUST NOT conter nem iniciar o sidecar do agente nativo.

#### Scenario: Bundle sem sidecar
- **WHEN** o app de release é montado
- **THEN** não existe `clovy-agent-runtime` no bundle e o app inicia sem esse processo
- Test: integration — build do bundle e listagem dos recursos; smoke nativo com lista de processos filhos

### Requirement: Histórico preservado
Sessões criadas pelo motor nativo SHALL continuar listáveis com itens e artefatos. Enviar ou repetir numa sessão nativa MUST usar o motor CLI padrão com o histórico como contexto. Execuções nativas que aguardavam o usuário MUST ser encerradas como canceladas por migração.

#### Scenario: Sessão antiga continua
- **WHEN** o usuário abre uma sessão nativa e envia uma mensagem
- **THEN** o histórico aparece completo e a resposta vem do CLI padrão com as mensagens anteriores no prompt
- Test: integration — banco com sessão nativa de fixture e CLI falso que grava o prompt

### Requirement: Configurações sem motor nativo
`llm_usage.chat` MUST aceitar apenas CLIs depois do corte; valores `clovy` e `endpoint` salvos MUST migrar uma vez para o CLI padrão. Endpoints registrados MUST continuar disponíveis para notas, limpeza e atividade.

#### Scenario: Perfil com chat no Clovy
- **WHEN** um perfil salvo com chat `clovy` abre após o corte
- **THEN** o chat passa ao CLI padrão uma única vez e a escolha de notas não muda
- Test: integration — `provider-settings.json` de fixture, carga, gravação e recarga
