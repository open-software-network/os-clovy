---
status: proposed
date: 2026-11-23
supersedes: ADR-0064 (interface Início); ADR-0063 (somente leitura para todos os clientes externos)
---

# Hoje é a entrada do desktop e o MCP externo usa aprovação por operação

## Contexto

As conversas passam para os clientes que o usuário já usa. O Clovy mantém
Atividade, notas, projetos, ditado e rotinas. A interface de conversa também
mostrava os resultados de rotinas e recebia os pedidos do broker de aprovação.
As sessões já gravadas incluem dados necessários para reproduzir esses resultados.

## Decisão

- A navegação inicial e os destinos antigos de chat passam para Hoje. Rotinas
  edita instruções e mostra o estado e resultado de cada execução no próprio fluxo.
  Sessões, itens e artefatos persistidos continuam disponíveis para seus consumidores.
- O relay `clovy-mcp` continua usando autenticação local por instalação. O host
  reconhece a conexão externa depois da autenticação e atribui autoridade por
  chamada. Clientes externos não podem escolher sessão, turno ou workspace do app.
- O usuário pode aprovar uma operação exata ou negar em Configurações → MCP. O
  broker invalida o pedido quando o cliente perde conexão ou autoridade. Antes
  de agir, o handler confere novamente a autoridade e o alvo.
- O catálogo externo inclui leituras existentes e handlers que podem operar com
  autoridade externa. Ferramentas que exigem um contexto interno ou uma política
  ainda indisponível nesse caminho não são anunciadas. O catálogo interno preserva
  as regras de cada consumidor.

## Alternativas

Manter a área de conversa apenas para resultados e aprovações preservaria os
controles antigos, mas deixaria esses fluxos acoplados ao composer e ao turno.
Empréstimo de um vínculo de turno para o MCP externo reutilizaria o caminho de
aprovação antigo ao custo de confundir a autoridade de duas origens. Um servidor
de rede exigiria outra configuração e ampliaria a exposição do transporte.

O contrato é definido na [change retirar-chat-do-clovy](../../openspec/changes/retirar-chat-do-clovy/).
O catálogo e o ciclo de aprovação são descritos em [MCP](../mcp-server.md).
