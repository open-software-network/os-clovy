# Proposal

## Why

O dono mostrou duas capturas do Meridian e relatou que a timeline do Clovy está diferente demais. A vista atual é uma faixa horizontal de 24 horas seguida de sessões por app; a referência mostra frentes de trabalho em uma grade temporal vertical, retomadas e detalhes laterais. O dono também pediu ícones reais em Principais apps.

## What Changes

- Substituir a apresentação principal da timeline por grade vertical de horários com cartões de frentes de trabalho, intervalos/retomadas e distribuição em colunas sem sobreposição ilegível.
- Ao selecionar um cartão, mostrar no painel lateral os horários, duração efetiva e o que foi feito; sem seleção, mostrar o panorama do dia.
- Manter busca, navegação entre dias, dados capturados e acesso ao resumo, com tokens, tipografia e idioma do Clovy; não copiar a cor das imagens.
- Resolver os ícones reais dos aplicativos instalados no macOS para Principais apps, sem substituir marcas diferentes por um ícone inventado.
- Completar os contratos de dados temporais necessários: frentes atuais só têm minutos agregados por hora, não intervalos precisos; nenhuma duração agregada pode virar um horário exato fictício.

## Capabilities

### New Capabilities

- `day-timeline-presentation`: apresentação temporal das frentes de trabalho e panorama/detalhes do dia, incluindo identidade visual real dos apps.

### Modified Capabilities

Nenhuma spec principal dessas capacidades foi sincronizada ainda. Esta change complementa `add-activity-intelligence` sem reabrir Jira/worklog.

## Impact

`src/components/activity-timeline/`, contratos em `src/lib/activity-timeline.ts` e `src/lib/day-intelligence.ts`, dados em `src-tauri/src/activity/timeline/` e `src-tauri/src/day_intelligence/`, traduções, estilos, testes comportamentais e documentação de atividade. Se necessário, adicionar comando macOS para resolver ícone local via bundle id. Sem migração/destruição do banco principal de notas, sem push ou instalação em /Applications. Referência visual: capturas do dono terminadas em `14.03.20.png` e `14.03.30.png` (2026-10-05).
