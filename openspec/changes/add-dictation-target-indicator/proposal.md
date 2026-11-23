# Proposal

## Why

O HUD de ditado não identifica visualmente onde o texto será colado. O indicador deve acompanhar o candidato durante captura e preservar o destino exato fixado ao parar, conforme ADR-0014.

## What Changes

- Mostrar ícone nativo local e nome acessível desde listening, com fallback honesto.
- Correlacionar metadados pelo takeId e congelar a apresentação no pin ao parar.
- Preservar indicador em processamento, colagem e erro; limpar entre tomadas.
- Manter controles, waveform, drag e identidade visual existentes.

## Capabilities

### New Capabilities

- `dictation-target-indicator`: apresentação persistente e fiel do aplicativo de destino de uma tomada.

### Modified Capabilities

Nenhuma.

## Impact

Helper Swift, coordenador Rust, HUD HTML/TypeScript/CSS, catálogos EN/pt-BR e testes. Reutiliza activity_app_icon e seu cache local; sem dependências, API, providers, permissões globais ou alteração do instante do pin. Prova nativa e aceite pertencem ao Main.
