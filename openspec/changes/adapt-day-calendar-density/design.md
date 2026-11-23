# Design

## Context

Ver proposal.md. A baseline é `9f3a7a1e`: `layoutDayGrid` usa 1,5 px/min, piso de 52 px e colisão pelos retângulos desenhados. `WorkstreamGrid` já dispõe de um scroller, título/metadados/notas, trilho de retomadas, nome acessível e callback de seleção. Apenas `ActivityTimelineView` e testes de layout consomem a função; o LSP não está configurado e as referências foram levantadas pelo Graft.

## Goals / Non-Goals

**Goals:** medir a altura útil da coluna do calendário; adaptar a escala com limites; compactar cartões sem texto cortado; calcular as colunas com a geometria efetiva; preservar o contrato temporal.

**Non-Goals:** mudar lateral, estatísticas, detalhes, geração por IA, títulos/IDs persistidos, dados, backend, provedores, paleta, fontes ou dependências. Não copiar código/assets da referência. Não implementar a animação tutorial do GIF.

## Decisions

- Medir o scroller/contêiner com os padrões existentes e `ResizeObserver`, mantendo a medição fora do conteúdo cuja altura será alterada. Uma altura ainda desconhecida usa uma geometria inicial estável; não pode produzir NaN ou um ciclo observador→layout→observador.
- Adaptar a escala a partir da janela temporal e dos blocos ocupados/ociosos. Usar limites explícitos comparáveis ao comportamento observado na referência (1–9 px/min nas horas ativas), sem crescimento ilimitado e sem forçar um dia longo a caber. Preservar horas locais, DST, compactação de ociosidade e indicador de agora.
- Separar a escala do retângulo mínimo/colunas: primeiro determinar a escala; depois distribuir os cartões considerando sua altura efetiva e um intervalo visual entre cartões. Não trocar simplesmente colisão por pixels por colisão cronológica, pois isso esconderia texto em cartões curtos.
- Usar o token `--control-xs` de 22 px como referência do piso compacto, respeitando fontes/line-height e variantes da interface. Divulgar progressivamente título, metadados e notas em patamares derivados do espaço real; nunca deixar meia linha aparecer. Preservar o nome acessível completo e a seleção já existente, sem mexer em seu destinatário lateral.
- Manter os dados e a união de retomadas do cálculo atual. Pausas nunca entram no tempo ativo; horas sem localização continuam explicitamente aproximadas.
- Migrar todos os consumidores da API de layout se sua assinatura mudar. Remover o caminho fixo obsoleto, sem aliases ou uma segunda política de layout em produção.

## Risks / Trade-offs

- Medir o canvas renderizado em vez da janela útil pode criar feedback de resize → medir o viewport independente e provar estabilidade após redimensionar.
- Cartões curtos podem perder contexto visual → preservar título e nome acessível, manter seleção/foco e revelar apenas linhas inteiras quando couberem.
- Reduzir piso sem recalcular faixas pode causar colisão → provar retângulos e seleção nos casos sequencial e sobreposto.
- Uma grade adaptativa muda a escala ao redimensionar → manter horários e tempo ativo explícitos, limitar crescimento e preservar rolagem em dias longos.
- CSS compartilhado pode afetar a lateral → tocar somente seletores `.day-grid-*`; componentes/seletores da lateral são proibidos.

## Migration Plan

Capturar RED comportamental antes da correção; implementar os três ajustes conjuntamente; atualizar testes e documentação pertinentes; concluir gate/build, smoke visual com dados sintéticos e revisão externa. Main confere a lateral preservada, valida independentemente, integra na `main` e observa o dev. Sem deploy de API ou publicação. Rollback é revert da entrega de frontend, sem migração de dados.
