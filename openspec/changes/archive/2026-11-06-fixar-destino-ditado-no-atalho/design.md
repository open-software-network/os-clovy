# Design

## Contexto

`FocusTargetController` (helper Swift) guarda `lastExternalApp` (atualizado a cada `didActivateApplicationNotification`) e `pinnedTarget` (só `NSRunningApplication`). Hoje:

- `markRecordingStarted` emite `listening_started` com o candidato atual.
- `rememberIfExternal` chama `emitCandidateTarget`, que emite `focus_target` com o candidato durante a escuta; o HUD (`src/hud.ts`) troca o ícone.
- `stopActiveRecording` chama `pinCurrentTarget()`, que primeiro atualiza o candidato com o app da frente e depois fixa. É aqui que a janela nova vence.
- `resetRecordingState` (chamado por `startRecording`) limpa o destino fixado.
- `paste` ativa o app fixado e só envia Command+V se ele estiver na frente (ADR 0014); não distingue janelas.

Prova de lógica já registrada: `.tmp/verify/dictation-start-target/investigation.json` (A no acionamento, B ao parar).

## Decisões

1. **Fixar no acionamento.** O destino é capturado quando o helper recebe o pedido de início da tomada de ditado (o ponto em que `startPending` vira verdadeiro), antes do pedido de microfone e antes de `resetRecordingState`. A captura atravessa o reset (passada explicitamente para a tomada, não limpa por ele) e fica associada ao `takeId`. Fixar em `markRecordingStarted` também é aceitável se o worker provar que não há janela de deriva entre o aperto e esse ponto; a escolha e o motivo vão para o relatório.
2. **Destino com janela e campo.** O destino guarda o `NSRunningApplication` e, se `AXIsProcessTrusted()`, o `AXUIElement` da janela focada (`kAXFocusedWindowAttribute` do app) e do elemento focado (`kAXFocusedUIElementAttribute` do sistema, aceito só se pertencer ao PID do app). Consultas AX com timeout curto (`AXUIElementSetMessagingTimeout`) para não atrasar o início da gravação. Sem Acessibilidade ou sem resposta, o destino fica só com o app.
3. **Sem deriva durante a tomada.** Com destino fixado, `emitCandidateTarget` não emite o candidato novo (ou reemite o fixado); `stopActiveRecording` não repina; `targetMetadata`/`targetBundleIdentifier` usam o fixado. `lastExternalApp` continua sendo atualizado para a próxima tomada. Rust/HUD só mudam se ainda trocarem o ícone a partir de outro evento.
4. **Restauração na entrega.** Em `paste`: ativar o app; se houver janela, `AXRaise` + `kAXMainAttribute` nela; se houver campo, `kAXFocusedAttribute = true`. Antes do Command+V, conferir app da frente e, se houver janela capturada, que a janela focada do app é a mesma (`CFEqual`). Falha ou prazo esgotado → `emitPasteTargetUnavailable` (texto fica no clipboard; Control+V continua com o último ditado). Restaurar o campo é melhor esforço: se a janela confere e o campo não aceita foco, cola na janela original e registra no log de eventos.
5. **Escopo.** Só a colagem automática do ditado. Control+V continua no foco atual. Teste de microfone e outras finalidades não capturam destino.

## Riscos

- Apps que não expõem AX (alguns Electron/jogos): cai para destino por app, como hoje, mas fixado no acionamento.
- Janela do próprio Clovy/HUD: continua ignorada pela lista de bundles do helper.
- Consulta AX síncrona no início: timeout curto, medido no relatório; não pode violar o requisito "Gravação começa no aperto do atalho".
- Elemento AX inválido depois de fechar a janela: tratado como destino indisponível, nunca como sucesso.

## Testes

- Lógica Swift extraída/real com adaptadores sintéticos (como na investigação): A no acionamento, B ativado durante a fala → destino e metadados continuam A; stop não repina; nova tomada captura de novo; cancelamento limpa.
- Mesma app, duas janelas sintéticas: janela diferente na frente → não cola, ou traz a original.
- Janela inválida → `paste_target_unavailable` sem Command+V.
- Testes Rust/HUD afetados, se mudarem.
- Prova manual do dono no app de QA: o cenário relatado (outra janela abre durante a fala).
