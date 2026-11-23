# Tasks

## 1. Preservação das autorizações

- [x] 1.1 Capturar RED comportamental do launcher normal que demonstra revogação de autorização em sandbox sem resetar TCC real; entregar recibo com asserção, teste e digest.
- [x] 1.2 Remover reset de TCC da inicialização normal e eliminar leitores/testes/documentação que prometem um walkthrough limpo em cada restart; comprovar GREEN do mesmo teste e executar a receita atualizada de startup sem revogação.

## 2. Identidade estável do app e dos helpers

- [x] 2.1 Integrar bundle local reconhecível e assinatura válida de desenvolvimento ao runner normal, mantendo HMR, argumentos, targets, perfis e isolamento; comprovar execuções do runner com fixture controlada, metadata do bundle e designated requirements estáveis antes/depois de rebuild.
- [x] 2.2 Reusar conteúdo/assinatura de helpers inalterados e manter parent legítimo autenticado; comprovar self-test pelo launcher exato e contraprova de parent/alias externo sem relaxar as regras.
- [x] 2.3 Atualizar a receita de dev e o mapa de concessão de app/helpers nos docs/DOX pertinentes, documentando seleção de identidade e primeira autorização; executar os comandos documentados no alcance autorizado e validar scenarios com Test.

## 3. Integração e comprovação nativa

- [ ] 3.1 Entregar GREEN focado, make verify e build de produção com recibos do snapshot final; report próprio com hashes, alcance e notProven, seguido do verify-cli-worker canônico.
- [ ] 3.2 Main comprovar pela UI/permissões reais o mesmo dev antes/depois de restart e rebuild, com autorização inicial real e execução das capacidades afetadas, conforme design.md; evidências de perfil e nenhum dado de produção tocado. Não marcar sem prova nativa completa.
- [ ] 3.3 Main concluir revisão independente, verify --record e aceite do conjunto, incluindo as pendências prévias de captura/MCP; manter explícitas etapas bloqueadas e não publicar sem autoridade.
