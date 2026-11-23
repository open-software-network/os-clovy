# Tasks

## 1. Preferências e migração

- [x] 1.1 Capturar RED comportamental de abertura sem ativação manual e carga do perfil legado desligado.
- [x] 1.2 Implementar padrões ligados e migração durável única antes de novas escolhas, preservando o restante da configuração.
- [x] 1.3 Comprovar opt-out independente dos dois switches em múltiplos reinícios e após gravação pelo outro controle, inclusive JSON parcial.

## 2. Serviços e documentação

- [x] 2.1 Comprovar início da captura com permissões controladas e bloqueio sem permissão, sem alterar barreiras existentes.
- [x] 2.2 Comprovar listener MCP real e ferramentas de sessão CLI compatível sem ativação manual, preservando autenticação e exclusão das ferramentas de atividade com captura desligada.
- [x] 2.3 Atualizar documentação/copy afetadas e remover testes do padrão obsoleto; executar gates aplicáveis e capturar build de produção.

## 3. Prova e aceite pelo Main

- [x] 3.1 Executar smoke nativo isolado: primeira abertura, MCP real, switches pela UI, reinícios e preferência persistida; declarar limite de permissões.
- [ ] 3.2 Revisar entrega, associar RED/GREEN e obter verificação independente com recibos antes do aceite.
- [ ] 3.3 Iniciar dev para o dono, observar janela/serviços, preservar dados e registrar destino dos recursos sem push.

Worker atualiza somente 1 e 2 quando comprovadas. Main atualiza 3 e mantém o ticket. Sem provas de captura macOS real se permissões não estiverem concedidas.
