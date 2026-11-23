# Tasks

## 1. Retirar os caminhos do chat

- [ ] 1.1 Em `src/app/`, sidebar, busca, notas, projetos, configurações, menu nativo e bandeja, remover a interface de chat e os disparos exclusivos. Abrir Hoje em todos os sistemas e normalizar abas, destinos gravados e eventos legados sem iniciar sessões. RED/GREEN de cold start, menu, atalho, navegação legada e troca de área. Remover código e traduções exclusivos depois de conferir os chamadores.
- [ ] 1.2 Em `src/components/routines/` e no shell, manter criação/edição e oferecer inspeção de estado, resultado e erro da execução sem composer. Provar execução existente e resultado, preservando os dados de sessões de versões anteriores.
- [ ] 1.3 Exercitar Hoje, timeline e blocos de agentes, configurações de Atividade, Notas de reunião, Projetos, Ditado e MCP. Conferir foco e navegação por teclado. Guardar capturas do app ou shim com fixtures.

## 2. MCP externo e aprovação

- [ ] 2.1 Em `src-tauri/src/mcp_server/channel/`, `protocol.rs` e `tools/`, dar contexto validado a chamadas de agentes externos com o transporte autenticado existente, identidade e escopo congelados pelo host. Reutilizar descoberta, permissões, schemas e handlers. Provar catálogo pelo relay stdio, leitura compatível e recusa de tentativa de usar turno/workspace interno.
- [ ] 2.2 Em `src-tauri/src/approval_broker/`, bridge Tauri e Configurações MCP, implementar pedidos externos com prévias e decisão única. Provar ausência de efeito antes de aprovar, execução única após aprovação, negação, expiração, desconexão e mudança de perfil, inclusive com duas conexões concorrentes. Testar uma consulta e uma escrita em notas ou memória pelo relay real com dados sintéticos e ações de maior efeito com fixtures controladas. Revalidar a autoridade depois de aguardar a decisão.

## 3. Requisitos, revisão e integração

- [ ] 3.1 Revisar proposal, design, specs e tasks afetados em `melhorias-agente-audio-automacao` e `antigravity-paridade-completa`. Retirar os requisitos de UI de conversa e aceite de paridade de chat que ainda estão pendentes. Manter provas concluídas, escopos de Atividade, provedores, rotinas, automação, permissões e catálogo de ferramentas. Atualizar documentação de MCP, rotinas e contexto com o comportamento entregue. Validar cada change afetada.
- [ ] 3.2 Rodar `pnpm check`, `pnpm typecheck` e `pnpm test` na box Linux. Rodar testes nativos focados do relay, broker, rotinas e Atividade no Mac, com banco/IPC de teste. Registrar plataforma, comando, saída, casos e limitações em `.tmp/verify/retirar-chat-do-clovy/`.
- [ ] 3.3 Revisar a árvore com o gate do harness. Conferir hash da entrega, aceitar, arquivar esta change, sincronizar o projeto e integrar localmente. Preservar provas e limpar apenas recursos comprovadamente usados pela entrega.

Recibos de 3.3 em `.tmp/verify/retirar-chat-do-clovy/review-pair/1/summary.md` e, para a contingência por exit 4, em `.tmp/verify/retirar-chat-do-clovy/manual-review/1/summary.md` (revisão manual).
