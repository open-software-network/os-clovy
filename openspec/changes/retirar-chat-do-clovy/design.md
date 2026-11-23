# Design

## Context

O shell monta `AgentWorkspaceRoute` tanto em Início quanto em Nova sessão. O mesmo destino é usado por atalhos, menus, projetos, notas, particularidades das configurações e histórico de Rotinas. A vista Hoje usa sua própria rota. O relay MCP externo é autenticado com o segredo da instalação e oferece um catálogo de leitura. Boa parte do catálogo de ferramentas depende da identidade de um turno, do perfil e do broker.

## Goals / Non-Goals

- Abrir em Hoje e retirar do produto todos os caminhos que apresentam o chat.
- Manter Rotinas com edição e resultados acessíveis sem conversa. Preservar o motor e os dados usados pelas rotinas e pelos agentes externos.
- Usar o relay já distribuído e o mesmo catálogo e handlers para as ferramentas compatíveis com clientes externos.
- Preservar dados, clientes, credenciais e preferências. O endereço do relay continua local.

## Decisions

1. Retirar a rota React de chat e as referências exclusivas dela. Normalizar destinos antigos de chat para Hoje sem disparar execução, incluindo atalhos e eventos nativos antigos. Manter o modelo e a persistência que o MCP, as rotinas ou o processamento de notas e Atividade usam. Eliminar controles exclusivos do chat de menus, onboarding, notas, busca, projetos, abas e configurações. Reaproveitar as superfícies e componentes existentes para Rotinas.
2. Usar identidade externa emitida pelo host após o handshake autenticado. Em cada chamada, capturar o perfil ou outra autoridade efetiva, validar argumentos e pré-condições de catálogo e limitar grants ao contexto da chamada. A identidade externa nunca recebe acesso ao workspace ou ao turno de uma conversa existente. Preservar o caminho vinculado e sua política para consumidores internos. Uma operação com efeito externo, destrutivo, de privacidade ou custo sempre exige aprovação explícita para o cliente externo. Reutilizar a lógica de cada handler e sua política de validação, inclusive os brokers de navegador e uso do computador onde aplicáveis.
3. Criar uma superfície de aprovações nas Configurações MCP e um indicador que leva a ela. Identificar ferramenta, alvo e perfil com prévia suficiente e texto seguro, dar uma decisão por pedido, revogar em cancelamento, desconexão, troca de perfil ou expiração. Revalidar alvo, vínculo e permissões antes do efeito. Funcionar com o app em segundo plano. Consultas externas atuais seguem funcionando com os mesmos parâmetros e filtros.
4. Remover dos planos antigos os itens ainda pendentes que exigem presença de chat interno, inclusive os cenários de paridade propriamente conversacional. Manter as provas já registradas como histórico e o trabalho pendente dos demais consumidores.

## Risks / Trade-offs

- Uma checagem que só testa `tools/list` não prova autorização. O teste completo usa relay real, roda a mesma chamada antes e depois de decidir, e confere o efeito na partição correta.
- Ferramentas com artefatos, mídia, navegador ou recursos dependentes de sessão precisam de contexto isolado emitido pelo host. Se o contexto ou a política não puder ser aplicado, a chamada falha sem efeito.
- A box Linux permite testes de frontend. Os caminhos nativos do MCP devem ser provados em plataforma compatível.
