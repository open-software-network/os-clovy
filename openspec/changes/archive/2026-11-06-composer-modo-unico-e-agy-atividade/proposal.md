# Proposal

## Why

O dono testou o app de QA e apontou três problemas:

1. Foi criado um segundo controle, o select "Aprovação de ferramentas", ao lado do campo de chat. Ele é visualmente ruim e duplica um controle que já existia: o seletor "Em sandbox / Sem restrições". "Sem restrições" deveria equivaler a dispensar todas as aprovações. Hoje os dois conceitos são independentes no backend (`approval_mode` × `safety_mode`).
2. O campo de chat da tela inicial ("Por onde o Clovy deve começar?") está alto demais (`min-height: 76px` no editor do hero). O dono quer o mesmo padrão compacto do campo de Rotinas em todos os campos de chat.
3. Ao escolher Antigravity em Atividade, aparece o erro de "JSON schema", embora o backend já suporte o Agy com `--json-schema`. O nível só é gravado pelo "Testar", e a mensagem não diz isso.

## What Changes

- Remover o select "Aprovação de ferramentas", com estado, textos, CSS e comando dedicados. O modo de aprovação de cada execução passa a ser derivado do modo de segurança: "Sem restrições" dispensa as aprovações do Clovy; "Em sandbox" mantém as aprovações. Execuções sem ninguém assistindo (rotinas) continuam sempre com aprovação.
- "Sem restrições" no Codex passa a usar acesso total (`danger-full-access`), por decisão explícita do dono. Isso derruba a proibição atual somente para esse modo.
- O seletor de modo continua só na conversa nova (decisão do dono).
- Campos de chat compactos como o de Rotinas: remover a altura mínima extra do hero e alinhar as medidas.
- Ao escolher em Atividade uma CLI sem nível medido, o Clovy roda o teste do provedor automaticamente e aplica a escolha se passar. Se faltar login no perfil do Clovy, a mensagem manda usar "Autenticar no Clovy"; erros de cota ou prazo são mostrados como tais, não como falta de JSON schema.

## Capabilities

### New Capabilities

Nenhuma.

### Modified Capabilities

- `cli-engine-permissions`: modo de aprovação derivado do modo de segurança; acesso total do Codex em "Sem restrições".
- `llm-providers`: teste automático ao escolher provedor de Atividade sem nível medido.
- Interface do composer (sem spec dedicada): campos compactos.

## Impact

Frontend: `src/components/agent/AgentWorkspace.tsx`, `src/styles/app.css`, `src/i18n/messages/agent.ts`, `src/lib/tauri.ts`, `agent-runtime-contract.ts`, `src/components/settings/LlmProvidersSection.tsx` (ou equivalente), testes em `src/test/`. Rust: `agent_sessions/{repository,api}.rs`, `chat_engine/policy.rs` (+ testes), `mcp_server/protocol.rs`, `lib.rs`, `llm/registry.rs`/`commands.rs`. Docs: ADR 0063, `docs/mcp-server.md`, `docs/llm-providers.md`, `AGENTS.md`.

## Non-goals

Não mudar o broker de aprovação além da origem do modo; não liberar `bypassPermissions`/`--yolo` no Claude/Antigravity; não mudar o ditado; não retomar a validação de paridade dos runtimes; não publicar release nem substituir o app real.
