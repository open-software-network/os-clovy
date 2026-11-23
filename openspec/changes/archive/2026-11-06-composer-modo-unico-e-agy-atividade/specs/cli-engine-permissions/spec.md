# Spec Delta

## MODIFIED Requirements

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

## ADDED Requirements

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
