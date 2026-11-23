# cli-engine-permissions Specification

## Purpose
Define como o modo de segurança da sessão e o tipo de turno viram flags de Claude Code e Codex, e como anexos e limites de tempo chegam ao CLI.

## Requirements

### Requirement: Modo de segurança chega ao CLI
Cada turno CLI SHALL carregar o modo de segurança da sessão (`sandboxed` ou `unrestricted`) e se é assistido, e MUST montar as flags pela tabela de `design.md` §3. `--allowedTools` MUST conter só padrões `mcp__clovy__*`. No Codex, `unrestricted` SHALL usar `danger-full-access` com `approval_policy = "never"`; `sandboxed` SHALL continuar em `workspace-write`.

#### Scenario: Sessão sandboxed no Claude
- **WHEN** um turno Claude inicia numa sessão `sandboxed`
- **THEN** o argv contém o modo de permissão padrão, o prompt-tool do Clovy (se S1 confirmou) e `--allowedTools mcp__clovy__*`, e nenhuma flag proibida
- Test: unit — argv montado por `policy.rs` para cada combinação; integration — CLI falso grava o argv

#### Scenario: Sessão unrestricted no Codex
- **WHEN** um turno Codex inicia numa sessão `unrestricted`
- **THEN** o sandbox é `danger-full-access`, sem aprovação nativa do Codex, e nenhuma outra flag proibida aparece
- Test: unit — argv por combinação

#### Scenario: Sessão sandboxed no Codex
- **WHEN** um turno Codex inicia numa sessão `sandboxed`
- **THEN** o sandbox continua `workspace-write` e `danger-full-access` não aparece
- Test: unit — argv por combinação

### Requirement: Flags de bypass proibidas
Flags que pulam ou pré-concedem aprovação (`--dangerously-skip-permissions`, `--allow-dangerously-skip-permissions`, `--permission-mode bypassPermissions`, `--dangerously-bypass-approvals-and-sandbox`, `--approve-for-me`) MUST NOT ser passadas em nenhum caso. `danger-full-access` MUST NOT ser passada, exceto no Codex em sessão `unrestricted`.

#### Scenario: Varredura das flags proibidas
- **WHEN** o argv é montado para todos os CLIs, modos e tipos de turno
- **THEN** nenhuma flag proibida aparece, e `danger-full-access` só aparece no par Codex + `unrestricted`
- Test: unit — teste de tabela sobre todas as combinações

### Requirement: Timeout de ferramenta coerente com o broker
O turno SHALL configurar o timeout de ferramentas MCP do CLI (Claude por ambiente, Codex por `tool_timeout_sec`) acima de `APPROVAL_TIMEOUT` e do polling de vídeo, nos valores confirmados pelo spike S3.

#### Scenario: Aprovação demorada
- **WHEN** o usuário aprova perto do fim de `APPROVAL_TIMEOUT`
- **THEN** o CLI ainda recebe o resultado da ferramenta, sem timeout do lado do CLI
- Test: integration — CLI falso com o timeout configurado e relógio controlado; smoke real na fase 3

### Requirement: Anexos e visão nos CLIs
Anexos de imagem de uma mensagem SHALL chegar ao CLI como imagem quando o CLI aceitar (Codex `--image <path>`, se S4 confirmar) e sempre pelo manifesto de caminhos no workspace. O manifesto MUST deixar de citar ferramentas de arquivo do Clovy.

#### Scenario: Imagem anexada no Codex
- **WHEN** o usuário anexa uma imagem numa sessão Codex
- **THEN** o argv contém `--image` com o caminho dentro de `attachments/` do workspace
- Test: integration — CLI falso grava o argv e o caminho existe

### Requirement: Clarificação do Claude pelo Clovy
Turnos Claude SHALL desabilitar a ferramenta interna de perguntas ao usuário quando S1 confirmar que ela não é respondível em `-p`, para que o modelo use `request_clarification`.

#### Scenario: Pergunta ao usuário
- **WHEN** o modelo precisa de uma resposta do usuário num turno Claude
- **THEN** a pergunta aparece como cartão de clarificação do Clovy
- Test: integration — CLI falso que chama `request_clarification`; smoke real na fase 3

### Requirement: Modo de aprovação derivado do modo de segurança
O modo de aprovação de cada execução SHALL ser derivado do modo de segurança gravado para essa execução: `unrestricted` dispensa as aprovações das ferramentas do Clovy; `sandboxed` mantém as aprovações. O app MUST NOT oferecer um segundo controle de aprovação. Execuções não assistidas MUST continuar exigindo aprovação, e pedidos de esclarecimento MUST continuar esperando o usuário.

#### Scenario: Conversa nova em "Sem restrições"
- **WHEN** o usuário começa uma conversa em "Sem restrições" e o agente pede uma ferramenta de escrita do Clovy
- **THEN** o pedido é aprovado automaticamente com recibo, sem cartão de aprovação

#### Scenario: Conversa em "Em sandbox"
- **WHEN** o agente pede uma ferramenta de escrita do Clovy numa conversa em "Em sandbox"
- **THEN** o cartão de aprovação aparece, mesmo que a conversa tenha um `approval_mode` legado igual a bypass

#### Scenario: Rotina em "Sem restrições"
- **WHEN** uma rotina sem ninguém assistindo roda em "Sem restrições"
- **THEN** o broker continua tratando a execução como não assistida, sem aprovação automática
