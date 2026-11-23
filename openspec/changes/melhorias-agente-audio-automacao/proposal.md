# Proposal

## Why

O dono aprovou corrigir os problemas de áudio, aprovações e ferramentas levantados na sessão do agente e substituir os motores de automação, construindo na agents-us e testando uso no Mac após integrar na main. Hoje há previews que sobrevivem ao descarte definitivo, controles Windows ocultos, operações nativas ausentes, instruções MCP incorretas e transporte de browser não ligado em produção.

## What Changes

- T2: controle de fontes no Windows, reconciliação explícita de previews descartados e exclusão da pausa no padding WASAPI; investigar endpoint e texto estranho sem atribuir causa a Groq ou portar Parakeet.
- T3: verificar a política efetiva por execução dos consumidores internos restantes, recibos verificáveis e contexto operacional vinculado. Aplicar as aprovações de clientes externos segundo `retirar-chat-do-clovy`. Escopos de resolução `session`/`always` serão recusados explicitamente.
- T4/T7: ferramentas de gestão do produto, transcrição integral e busca coerente, isolamento pela partição da conversa, atualização visível da UI e descoberta consistente de skills. Corrigir guia e causas de erro, sem adicionar correção manual de categorias ou mover escopo de memórias.
- T5: adotar agent-desktop no helper privado e agent-browser em transporte supervisionado; navegador próprio com perfil persistente e login separado. **BREAKING:** retirar compartilhamento de abas pessoais/pareamento de Browser use e os backends substituídos após cutover. Segundo plano por padrão, modo visível autorizado pelo dono quando a operação não tiver suporte background, nunca replay de efeito incerto ou elevação de privilégios do SO.
- T6: Vercel OAuth gerenciado com PKCE, refresh/desconexão e estado/origem reais. Aceite de cliente pelo provedor e consentimento são pré-requisitos externos explícitos; não reutilizar client_id alheio. Erros de integrações herdadas são atribuídos ao seu proprietário, sem trocar chaves nativas por inferência.
- T7: revisão integrada, scanner de segredos, provas e documentação; construir na box, integrar na main após revisão e executar uso nativo no Mac. Linux não é plataforma de produto: o baseline Rust já falha sem alterações por credentials/keyring condicionados a macOS/Windows; não portar o app para Linux para contornar isso.

## Capabilities

### New Capabilities
- `meeting-audio-integrity`: controles de fontes e consistência entre preview, final e tempo ativo.
- `agent-tool-management`: gestão nativa autorizada de conversas, projetos, notas, rotinas, memória, atividade, ditado, gravações e skills.
- `browser-control`: navegador dedicado persistente, adapter privado, identidade e lifecycle do controle.
- `managed-mcp-oauth`: OAuth gerenciado, Vercel, desconexão e diagnóstico por proprietário.

### Modified Capabilities
- `cli-approval-broker`: vigência por execução, decisão única e proveniência dos recibos.
- `computer-use-availability`: agent-desktop, plataformas e modo background/visível sem perder isolamento.
- `clovy-mcp-server`: contexto bound, partição e instruções/catálogo reais.

## Impact

Rust: áudio/preview, agent_sessions, approval_broker, MCP/tools/protocol, repositories/commands, computer_use/helper, browser/broker/proxy, agent_mcp/gateway e lifecycle. React: configurações/capacidades, editor de notas, histórico de Rotinas e sincronização de mutações nas áreas disponíveis. Packaging: engines/Chrome pinados por origem/commit/digest e builds por plataforma; retirar dependências exclusivas antigas sem apagar utilidades compartilhadas.

As changes `ocultar-icone-dock` e `ditado-app-alvo-e-contexto` continuam donas de T1; não duplicar suas tarefas. Esta change cobre T2–T7, com o escopo de chat interno encerrado por `retirar-chat-do-clovy`, e depende dos contratos completos registrados em `docs/qa/agent-runtime-backlog.md` e `docs/qa/windows-audio-backlog.md`. Nenhuma funcionalidade é considerada provada apenas por README, catálogo registrado ou transporte falso.
