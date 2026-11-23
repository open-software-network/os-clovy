# Design

## Context

Ver proposal.md. A UI atual (`ActivityTimelineView.tsx`) monta `TimelineDayStrip` horizontal e `TimelineSessionList`; detalhes substituem estatísticas à direita. Os contratos de captura já têm timestamps e bundle ids. `WorkstreamDto` tem título/resumo/minutos e `hours[{hour,minutes,note}]`, sem sittings exatos. A referência do dono é o Meridian `DayTaskColumn` + `DayTaskDetailPanel`: cartão da primeira à última retomada, pequenos marcadores dos sittings e duração ativa independente da altura do cartão.

## Goals / Non-Goals

**Goals:** grade e seleção legíveis em janela de desktop e larguras menores; dados temporais honestos; ícones locais identificados corretamente; preservar navegação, busca, resumo e estados de privacidade.

**Non-Goals:** Jira/worklog, publicidade/GitHub do Meridian, nova paleta, permissões automáticas do macOS, instalação em /Applications, alteração do banco principal ou capturar mais dados do que o usuário habilitou.

## Decisions

- Reutilizar frentes reais da inteligência do dia e sessões capturadas; não tratar nomes de apps como se fossem tarefas inferidas. Completar a associação temporal a partir das fontes medidas quando necessária. Horas legadas permanecem explicitamente agregadas; migrar dados de atividade de forma aditiva e manter os leitores/consumidores coerentes.
- Grade vertical com eixo compartilhado, cartões em colunas de conflitos temporais, marcadores de retomadas e indicador de agora. Duração efetiva vem dos dados, nunca da altura/intervalo total do cartão. Limites são locais do dia, inclusive horário de verão.
- Sem seleção: estatísticas de foco, apps e categorias. Com seleção: detalhes da frente e seus períodos/atividades. Busca conserva acesso ao detalhe da sessão.
- Resolver ícones por bundle id através das APIs locais do macOS, com cache e payload limitado; nome só pode ajudar na resolução quando não houver identificador e não criar colisão entre apps homônimos. Evitar varrer /Applications a cada render/atualização. Fallback neutro só quando o app/ícone realmente não estiver disponível.
- Prova: RED comportamental antes do fix; gate existente; smoke visual no app debug com dados -dev, seleção/retomadas, navegação e dois ícones reais. Fixture de grade cobre sobreposição e legado; não vale como prova do ícone nativo ou da captura real.

## Risks / Trade-offs

- A granularidade por hora existente não comprova minuto exato. Completar o contrato temporal sem inventar dados é parte do fix, não um detalhe cosmético.
- Dias sem inteligência ainda precisam mostrar a captura. Estado sem provedor explica que as frentes não foram geradas, sem bloquear sessões.
- O processo `pnpm tauri:dev` não é listado pelo Computer Use nesta máquina. Prova nativa automatizada pode usar a cópia debug com id `.proof` conforme AGENTS.md, nunca a instalação real; somente uma prova/app por vez. O Main mantém a sessão dev do dono e controla quando pode ser substituída para a prova.
