# Publicar release somente para macOS quando explicitamente selecionado

## Why

O dono aprovou publicar0.1.6 somente para Mac porque o repositório não tem o certificado Windows exigido pela implementação atual. A seleção já foi construída e revisada no commit ed74fe3e. Este registro fecha o contrato/documentação dessa mudança pequena e aprovada; não inventa aprovação anterior de um plano formal nem qualificação Windows.

## What Changes

- Seleção explícita all ou macos, padrão all; escolha validada uma vez e compartilhada pelos jobs e manifesto.
- Mac-only publica somente o DMG, tar assinado e manifesto Darwin; nenhum Windows fictício ou versão antiga renomeada.
- Falhas/cancelamento/artefatos selecionados ausentes continuam impedindo publicação; dry run continua sem publicar.
- O contrato de assinatura preserva a chave do updater e a política Apple, e corrige a frase canônica obsoleta de Windows sem Authenticode para a exigência já existente de certificado válido quando Windows é selecionado. Nenhum código de autenticação/certificado é alterado nesta mudança.

## Capabilities

### New Capabilities

Nenhuma.

### Modified Capabilities

- `release-distribution`: seleção das plataformas, pacotes por modo e credenciais somente da plataforma selecionada.

## Impact

Workflow release.yml, gerador release-manifest.mjs, dois testes de release, guias Mac/Windows e AGENTS já entregues. Código do app, chave pública, canal de atualização e regra de aceite do dono não mudam. A variável temporária será restaurada depois do run42306597955. Captura real/Meet, inferência paga, paridade e aceite Windows continuam nos registros próprios, sem arquivamento indevido.
