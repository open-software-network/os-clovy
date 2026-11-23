# Proposal

## Why

Depois da comparação com a referência animada, o dono autorizou adotar escala adaptativa, cartões curtos compactos e colunas coerentes com essa geometria. O calendário atual usa 1,5 px/min e um piso de 52 px; esta change altera apenas sua apresentação, preservando informações, seleção e legibilidade.

## What Changes

- Adaptar a escala das horas ativas à altura útil do calendário, com limites para dias muito curtos e sem comprimir dias longos a ponto de perder leitura.
- Apresentar cartões curtos como pílulas e revelar título, horários, tempo ativo e notas progressivamente quando houver espaço; manter o nome completo e os dados essenciais acessíveis.
- Calcular colunas após conhecer a escala e a altura efetivamente desenhada, evitando sobreposição física e mantendo cartões sequenciais na mesma coluna quando couberem.
- Preservar retomadas, pausas, duração ativa, dados aproximados, busca, navegação e seleção.
- Não alterar o painel lateral, os provedores, a geração/agrupamento das frentes, os bancos, as permissões ou as credenciais. Não mudar identidade visual, cores ou tipografia do Clovy.

## Capabilities

### New Capabilities

- `day-timeline-presentation`: requisitos adicionais de escala e densidade do calendário. Reutiliza o nome da capability definida em `align-day-timeline-with-reference`; o inventário principal ainda está vazio e aquela delta não foi sincronizada. Seus requisitos anteriores continuam válidos.

### Modified Capabilities

Nenhuma capability principal existente.

## Impact

`src/components/activity-timeline/day-grid-layout.ts`, `WorkstreamGrid.tsx`, integração estritamente necessária em `ActivityTimelineView.tsx`, seletores do calendário em `src/styles/activity-timeline.css`, testes de layout/grade e documentação da timeline. Sem alterações Rust, API, dependências, lateral ou geração por IA. Provas em navegador isolado com dados sintéticos, além da observação do dev atualizado; sem push/deploy.
