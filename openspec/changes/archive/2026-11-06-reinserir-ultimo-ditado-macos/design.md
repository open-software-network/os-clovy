# Design

## Context

Ver `proposal.md` para motivação e `specs/dictation/spec.md` para critérios. O helper Swift registra os atalhos por Carbon em `ShortcutKeyMonitor`; o fluxo normal usa `DictationController.paste` e `PasteboardInserter`, que restaura o clipboard 700 ms depois do atalho sintético. Rust já arbitra cancelamento e entrega em `src-tauri/src/dictation.rs`, com perfil fixado antes do processamento. ADR-0014 continua valendo para a entrega automática; ADR-0049 continua proibindo entrega de tomada cancelada.

A referência Liv Light guarda `state.lastTranscription` e registra Control+V somente no Mac. Seu handler espera 150 ms e usa inserção direta, não o clipboard. A referência não é alvo de edição.

## Goals / Non-Goals

**Goals:** recuperação manual independente do clipboard, texto Unicode completo, destino atual e isolamento de perfil.

**Non-Goals:** mudar entrega automática, adicionar opção de clipboard/histórico, copiar transcrições do disco, implementar Windows, retomar gateway ou alterar TCC. Uma inserção emitida não equivale a confirmação de consumo pelo app de destino.

## Decisions

### D1: Um último texto elegível e qualificado pelo perfil, somente em memória

Rust guarda uma única entrada com perfil/texto, junto do estado de ditado existente. Atualiza somente depois que o resultado final, não vazio e não quarentenado, vence a autorização de entrega. Uma falha posterior da colagem não invalida a recuperação. O comando de voz que dispara ação de agente não é convertido em nova ação por esse atalho. A escolha de manter a autoridade em Rust, em vez de um cache cego no helper, evita vazamento entre perfis e reaproveita o claim de cancelamento. Não consultar SQLite nem criar tabela.

### D2: Control+V no mesmo monitor nativo, fora da máquina de gravação

Reutilizar o registro Carbon e suas rotinas de suspensão/retomada durante captura. O callback da reinserção não inicia/para gravação, não aquece provedores e não falsifica eventos de uma tomada. Um pressionamento gera uma reinserção; segurar a tecla não gera uma rajada. Conflito com atalhos configurados de gravação dá prioridade aos existentes e emite o erro estável de registro apropriado.

O helper sinaliza a solicitação pelo IPC existente; Rust só devolve o último texto quando o perfil atual coincide. O helper correlaciona a resposta à solicitação de teclado e ao app atualmente focado, sem reutilizar o paste target antigo e sem aceitar uma resposta atrasada depois de perder esse destino. Não acrescentar API MCP nem expor um comando frontend irrestrito de digitação.

### D3: Inserção Unicode, sem tocar o clipboard

Aguardar a liberação dos modificadores com prazo limitado e revalidar foco/Acessibilidade antes de emitir pares de CGEvent Unicode sem flags de modificadores. Dividir textos grandes em blocos sem quebrar pares UTF-16; preservar acentos, emoji e novas linhas. Não usar `PasteboardInserter.paste`, porque ele escreve no clipboard e ativa o destino antigo.

### Approaches

- **Selecionada, conforme o comportamento pedido da referência:** memória + Control+V independente + digitação Unicode. Preserva o clipboard e permite recuperação em outro campo.
- Manter sempre a transcrição no clipboard: rejeitada, substitui o clipboard normal e não oferece o atalho independente pedido.
- Apenas abrir o histórico para copiar: rejeitada, exige navegação e não replica a recuperação imediata.

## Risks / Trade-offs

- [Apps podem rejeitar eventos Unicode] → provar em campo nativo e web; reportar o limite, sem fallback de clipboard silencioso.
- [Resposta IPC/foco pode ficar obsoleta] → correlação e revalidação antes da inserção, sem ativar outro app.
- [Modificador ainda pressionado ou key-repeat] → esperar liberação com prazo e uma ação por pressionamento.
- [Texto de outro perfil ou tomada cancelada] → armazenamento qualificado, atualização no claim existente e testes negativos.
- [Registro global consome Control+V] → macOS apenas, preservar Command+V e reportar conflitos sem remover controles alheios.

## Migration Plan

Implementar em checkout isolado a partir do checkpoint QA `72380fc9bd24c172724bc05b1c42376d57ef441f`, sem absorver alterações não relacionadas do checkout de revisão. Testar primeiro as transições e compilar o helper no Mac. Provar em app de QA com identidade/dados próprios; não substituir o app real nem usar dados pessoais. Não há migração de schema ou configuração. Reversão consiste em retirar esta mudança e seus registros, mantendo o fluxo automático anterior.
