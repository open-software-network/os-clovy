# Proposal

## Why

No macOS, o destino do ditado só é fixado quando o usuário solta o atalho (`stopActiveRecording` → `pinCurrentTarget`). Enquanto ele fala, o helper acompanha qualquer app que ganhe foco (`rememberIfExternal` → `focus_target`). Se outra janela surgir na frente durante a fala (um agente que abre uma janela, um pop-up, um clique), o ícone do indicador muda e o texto vai para essa janela em vez do campo em que o usuário estava ao apertar o atalho. O usuário testou no app de QA e confirmou a falha.

## What Changes

- Capturar o destino da tomada no acionamento do atalho de ditado: app, janela e campo focados naquele instante, quando a Acessibilidade permitir.
- Manter esse destino durante a fala, a finalização e a entrega da tomada: o indicador não troca de ícone, a limpeza recebe o mesmo app e o stop não fixa o destino de novo.
- Na entrega, reativar o app, trazer a janela original para a frente e restaurar o foco do campo original antes do Command+V; colar só se o app e a janela da frente forem os da tomada.
- Destino que não existe mais ou não pode ser restaurado não autoriza colar em outro lugar: segue a recuperação existente (texto no clipboard, aviso e Control+V).
- Fora de uma tomada, nada fica travado: o candidato continua acompanhando o foco normalmente e cada nova tomada captura o próprio destino.

## Capabilities

### New Capabilities

Nenhuma.

### Modified Capabilities

- `dictation`: destino da colagem automática fixado no acionamento do atalho, com janela e campo, durante toda a tomada.

## Impact

Helper Swift `src-tauri/native/mac-dictation-helper/main.swift` (`FocusTargetController`, início/stop da tomada, `paste`). Rust `src-tauri/src/dictation.rs` e `src/hud.ts` só se precisarem ignorar atualizações de destino da tomada já fixada. Testes focados, `docs/dictation.md`, `docs/adr/0014-pinned-dictation-paste-target.md` (atualizar o momento da fixação) e `AGENTS.md`. Sem dependência nova, permissão nova, schema ou mudança no Windows/Linux.

## Non-goals

Não impedir que outras janelas abram; não garantir que todo app aceite restauração de foco por Acessibilidade; não mudar o Control+V (continua usando o foco atual); não mudar prazos de colagem ou restauração do clipboard; não retomar a paridade dos runtimes, publicar release ou substituir o app real.
