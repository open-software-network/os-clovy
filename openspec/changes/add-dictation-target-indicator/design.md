# Design

## Context

Ver proposal.md. FocusTargetController mantém NSRunningApplication e fixa ao parar. O HUD hoje ignora focus_target e colapsa para spinner. O tamanho nativo é medido pelo DOM, não por src-tauri/src/hud.rs (arquivo inexistente nesta base).

## Goals / Non-Goals

**Goals:** manter a identidade por tomada sem alterar a autoridade do helper; preservar controles e geometria dinâmica.

**Non-Goals:** resolver ícones Windows, mudar colagem, providers, dados ou permissões; abrir app nativo no worker.

## Decisions

- Payload aditivo target: {app, bundleIdentifier, processIdentifier} ou null. listening_started inclui candidato; focus_target só durante captura e com takeId; finalizing_transcript inclui snapshot fixado antes do teardown. paste_target inclui a mesma identidade. O PID descreve a instância; nunca comanda colagem.
- Reutilizar activityAppIcon/activity_app_icon e cache existente por bundle id. Evita PNG em todos os eventos de áudio e uma segunda implementação AppKit. Falhas não bloqueiam o fluxo; geração local invalida promises antigas.
- O coordenador aplica correlação e serialização também a focus_target. Snapshot para HUD tardio conserva metadados e fase sem transformar evento visual em início/fim de captura.
- Indicador independente do viz, com separador e tokens existentes. Não participa do fade de morph; spinner mantém seu lugar próprio.

## Risks / Trade-offs

- Foco e stop concorrentes → ordem do run loop do helper, pin antes do teardown, consumidor congela no snapshot.
- Promise lenta, cancelamento e novo take → geração e identidade antes de aplicar resultado.
- Ícone por bundle não distingue instalações do mesmo bundle → serve só apresentação; instância exata continua no helper e PID nos metadados.
- Testes de DOM não provam AppKit, Fn ou paste → Main executa prova nativa isolada; worker entrega método e build sem abrir app.

## Migration Plan

Protocolo local aditivo: eventos antigos continuam com fallback. Sem migração de dados ou Clovy API. Reversão pelo commit da entrega.
