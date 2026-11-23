# Design

## Context

Ver proposal.md. `ActivitySettings` usa serde(default), enabled=false e mcp_server=false. `activity::start` carrega o JSON antes do loop de captura; `mcp_server::setup` reconcilia o listener com o mesmo estado compartilhado. O formato atual não distingue false padrão de desligamento explícito. O dono autorizou ativação única em perfis existentes; worker OMP escolhido.

## Goals / Non-Goals

**Goals:** uma carga autoritativa das preferências, migração durável única, switches independentes, sem regressão do opt-out.

**Non-Goals:** bypass de permissões, configuração global de clientes MCP externos, autenticação de apps externos ou OS Accounts, novos coletores, chaves ou keychains. Sem push, branch nova ou captura de dados reais durante teste.

## Decisions

- Manter o JSON e o save atômico existentes; adicionar somente identificação durável suficiente para distinguir perfil pré-upgrade de perfil já atualizado. Não usar inicialização React para resetar switches.
- Migrar e persistir uma vez antes de consumir novas escolhas. Todos os caminhos de save devem preservar a identificação do upgrade e a escolha independente do outro serviço; evitar salvar DTO parcial apagando estado interno.
- Configuração nova/campos ausentes ligam ambos; perfis pré-upgrade ligam ambos uma vez. Após upgrade false é decisão explícita e nunca é substituído em boot. O executor escolhe representação mínima e explica falhas de persistência, sem erro silencioso que reaplique upgrade e perca opt-out.
- Manter o bootstrap e o protocolo local atuais. Não remover avisos legítimos quando o usuário efetivamente desliga o MCP; eliminar somente copy/testes que afirmem o padrão antigo.
- Representação adotada: campo `defaultsVersion` (u32) em `activity-settings.json`; ausente vale 0 (perfil pré-upgrade) e `DEFAULTS_VERSION = 1`. `settings::load_at_startup` liga captura e MCP uma vez quando a versão é menor, grava e devolve as preferências já marcadas; se a gravação falhar (aviso no stderr do processo, padrão `eprintln!` já usado no app; o app não tem subscriber de `tracing`), o marcador fica em memória e a próxima gravação bem-sucedida de qualquer switch o persiste. `activity_save_settings` preserva `mcpServer` e `defaultsVersion` atuais; o switch do MCP grava só `mcpServer`.

## Risks / Trade-offs

- [False legado indistinguível de opt-out] → ativação única expressamente autorizada pelo dono em 2026-10-07; todos os futuros opt-outs preservados.
- [Gravação pelo frontend omite marcador] → regression por ciclo real de carga/save/recarga e comandos dos dois switches.
- [Captura automática sem permissão] → engine mantém barreira de permissões e estado visível ligado/bloqueado; nenhuma concessão de privacidade pelo agente.
- [Dados do dono] → Main realiza smoke apenas em cópia -dev com identifier alternativo compilado, HOME normal e lsof; worker não abre app nativo. Uso dev do dono é launch final, sem seeding ou controle negativo.

## Migration Plan

1. RED comportamental de carga de perfil novo/legado e efeito nos serviços; implementar migração persistente e preferências.
2. GREEN de perfis novos, JSON parcial, legado, dupla recarga após opt-out e independência. Serializar Rust/build para reutilizar target existente.
3. Smoke Main: copiar perfil dev sem socket vivo, compilar identifier alternativo, abrir bundle exato. Observar captura ligada ou bloqueada por permissões, MCP pronto e tools/list real. Desligar switches pela UI, reabrir e conferir persistência; reativar e conferir novo reinício. Desfazer autostart somente se criado por este run.
4. Revisão proporcional e verificador em sessão distinta; registrar alcance/limites. Abrir dev do dono após integração local. Sem deploy/push solicitado.

Se permissões do app isolado não existirem, o smoke prova início habilitado/bloqueado e MCP real; frames de captura são comprovados no engine com plataforma controlada, não declarados como captura real do macOS.
