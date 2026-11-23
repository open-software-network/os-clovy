# Proposal

## Why

O histórico do Ditado mostra o mesmo ícone de microfone em toda linha, embora o HUD do ditado já saiba e mostre o app em que o texto foi colado. E os relatórios por hora e o resumo do dia são gerados só a partir da atividade capturada, sem o que o usuário falou ao ditar naquele período, que diz muito sobre o que ele estava fazendo.

## What Changes

- Cada ditado passa a gravar o app de destino (bundle id fixado pelo helper quando a gravação para).
- O histórico do Ditado mostra o ícone desse app no lugar do microfone. Ditados antigos, sem app, e apps sem ícone mantêm o microfone.
- O relatório por hora e o resumo do dia recebem, como seção extra dos dados, os ditados feitos naquele período (horário, app quando conhecido, trecho do texto). Sem ditados no período, o prompt fica idêntico ao de hoje.
- Ditados sozinhos não disparam relatório nem resumo: as regras atuais de "sem atividade, sem chamada" continuam iguais.

## Capabilities

### New Capabilities

### Modified Capabilities
- `dictation`: novo requisito de registrar o app de destino e mostrá-lo no histórico.
- `day-intelligence`: novo requisito de usar os ditados do período como contexto do relatório por hora e do resumo do dia.

## Impact

- Banco: migração 54 adiciona coluna anulável `app_bundle_id` em `dictation_history`, sem backfill.
- Rust: escrita do histórico em `dictation.rs` e `db/repositories.rs`; DTO em `domain/types.rs`; `day_intelligence/{sources,hour,summary,pipeline}.rs` com nova consulta por intervalo de tempo.
- Frontend: `DictationHistoryView.tsx` usa o `AppIcon` já existente (comando `activity_app_icon`, o mesmo do HUD); tipo em `src/lib/tauri.ts`; CSS do chip.
- Privacidade: o texto dos ditados do período passa a ir para o provedor de atividade, dentro da cerca de dados já usada; limitado aos 7 dias de retenção do histórico. Documentar em `docs/day-intelligence.md`.
- Windows: o helper não envia bundle id no fim da gravação; as linhas continuam com o microfone e os ditados entram no contexto sem app.
