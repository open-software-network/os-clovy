# Proposal

## Why

O dono relata que precisa conceder permissões repetidamente para trabalhar em dev. O launcher atual apaga Acessibilidade do helper de computer use e Screen Recording do app em toda inicialização, e o runner executa um alias de Cargo sem bundle de app reconhecível para concessão manual.

## What Changes

- Preservar autorizações existentes no caminho normal `pnpm tauri:dev` / `make dev`; retirar o reset automático de TCC.
- Dar ao app dev e aos helpers uma identidade e caminhos estáveis, com bundle local reconhecível pelos Ajustes do macOS, sem exigir instalar o produto de release.
- Evitar reconstrução, troca de identidade ou assinatura desnecessária de helpers inalterados.
- Manter estados de permissão reais e explicar a autorização inicial ou uma mudança efetiva de identidade, sem fingir concessão.
- Comprovar reinício e recompilação no fluxo normal, além dos testes de regressão do launcher e das fronteiras de autenticação dos helpers.

## Capabilities

### New Capabilities

- `dev-runtime`: inicialização do desenvolvimento nativo com identidade estável, permissões preservadas e isolamento dos dados de produção.

### Modified Capabilities

Nenhuma; `openspec list --specs` não encontrou specs principais neste fork.

## Impact

Launcher e runner em `scripts/`, preparação/assinatura/staging dos helpers em `src-tauri/build.rs` e scripts pertinentes, validação da identidade do parent dos helpers se o caminho mudar, testes e documentação do dev. O ticket existente continua responsável pela entrega completa de captura/MCP e do dev; os gates anteriores comprovam apenas o snapshot anterior.

Não alterar autorizações internas de tarefas, rollout, billing, serviços de produção, o banco de TCC, SIP, senhas ou configuração do Keychain. Nenhum push, instalação de release, nova branch ou worktree. Uma autorização inicial exigida pelo macOS não será prometida como automática.
