# Proposal

## Why

A colagem automática do ditado pode enviar Command+V sem o texto entrar no campo; o clipboard anterior volta após 700 ms. O usuário pediu a recuperação independente já existente na referência local Liv Light: Control+V reinsere o último ditado no campo atualmente focado.

## What Changes

- Adicionar Control+V global somente no macOS para reinserir o último texto de ditado válido desta execução, sem depender do clipboard.
- Manter esse texto apenas em memória; uma nova transcrição válida substitui a anterior, mas silêncio, cancelamento e resultado rejeitado não o substituem.
- Usar inserção Unicode direta no foco atual após soltar os modificadores, sem reativar o destino antigo do ditado.
- Preservar Command+V, a restauração atual do clipboard, a autoridade/cancelamento das tomadas e os atalhos de captura existentes.
- Tratar falta de Acessibilidade e conflito de registro sem colagem silenciosa nem nova permissão de Input Monitoring.

## Capabilities

### New Capabilities

Nenhuma.

### Modified Capabilities

- `dictation`: recuperação explícita do último ditado por Control+V no macOS, independente do clipboard.

## Impact

Helper Swift `src-tauri/native/mac-dictation-helper/main.swift`, coordenação Rust do ditado apenas se necessária para manter o vínculo e isolamento existentes, testes focados e `docs/dictation.md`/`AGENTS.md`. Sem dependência nova, mudança de schema, configuração de clipboard, port Windows/Linux ou trabalho de gateway. Referência somente leitura: `/Users/guilhermevarela/Documents/Projetos/Desenvolvendo/liv-light/src/main/global-shortcut.ts`.

## Non-goals

Não alterar a colagem automática ou seu prazo; não garantir que outros apps aceitem eventos Unicode; não persistir a memória após sair/reiniciar; não copiar histórico pessoal ou credenciais; não retomar a paridade dos runtimes, publicar release ou substituir o app real.
