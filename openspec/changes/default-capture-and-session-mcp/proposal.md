# Proposal

## Why

Captura e MCP local usam `false` como padrão, deixando sessões CLI sem ferramentas do Clovy e exigindo ativação manual. O dono quer ambos iniciados automaticamente e opt-out persistente, inclusive uma ativação única dos perfis existentes autorizada em 2026-10-07.

## What Changes

- Captura de atividade e servidor MCP local ligados por padrão ao abrir o Clovy.
- Migração única dos perfis anteriores, ativando ambos sem reativar futuras desativações explícitas.
- Independência dos controles: desligar captura não desliga MCP de notas/memórias; desligar MCP não desliga captura.
- Persistência de escolha entre reinícios, inclusive preferência salva antes de fechar após o upgrade.
- Permissões do macOS, pausa manual, filtros de privacidade, autenticação local e isolamento de perfis permanecem obrigatórios.

## Capabilities

### New Capabilities

O inventário principal ainda está vazio; reutilizamos os caminhos da change `add-activity-intelligence`, com requisitos de inicialização que substituem seus padrões desligados nesta entrega.

- `activity-capture`: inicialização opt-out, migração única e preservação da escolha de captura.
- `clovy-mcp-server`: inicialização opt-out, migração única e ferramentas locais em sessões compatíveis sem ligar manualmente.

### Modified Capabilities

Nenhuma spec principal existente.

## Impact

Configuração `src-tauri/src/activity/settings.rs`, carga na inicialização da atividade, reconciliação do MCP local, testes de ciclo de vida e documentação/UI que descrevem os padrões. Sem alterar OS Accounts, provedores, credenciais, permissões do sistema ou clientes externos. A imagem Bops/Orgo citada antes foi desconsiderada pelo dono em 2026-10-07 e está fora desta entrega.
