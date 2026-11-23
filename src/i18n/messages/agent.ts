import { defineMessages } from "../define";

export default defineMessages({
  en: {
    // Hero
    "agent.hero.greeting1": "What can Clovy do for you?",
    "agent.hero.greeting2": "What should we work on?",
    "agent.hero.greeting3": "Where should Clovy start?",
    "agent.hero.greeting4": "What can Clovy take off your plate?",
    "agent.shortcut.recapNotes.title": "Recap my notes",
    "agent.shortcut.recapNotes.description": "What happened, what got decided, what's still open.",
    "agent.shortcut.research.title": "Research a topic",
    "agent.shortcut.research.description": "Get a short, sourced write-up on anything.",
    "agent.shortcut.summarizeFile.title": "Summarize a file",
    "agent.shortcut.summarizeFile.description": "Pick a document and get the key points out of it.",
    "agent.shortcut.healthCheck.title": "Check my Mac's health",
    "agent.shortcut.healthCheck.description": "Disk, memory, and login items that need attention.",
    "agent.shortcut.draftFollowUp.title": "Draft a follow-up",
    "agent.shortcut.draftFollowUp.description":
      "Turn your latest meeting note into a follow-up message.",
    "agent.shortcut.findFile.title": "Find a file",
    "agent.shortcut.findFile.description": "Describe what you remember; Clovy tracks it down.",
    "agent.shortcut.analyzeSpreadsheet.title": "Analyze a spreadsheet",
    "agent.shortcut.analyzeSpreadsheet.description":
      "Key figures, trends, and oddities from a CSV or sheet.",
    "agent.shortcut.searchNotes.title": "Search my notes",
    "agent.shortcut.searchNotes.description": "Find where something came up across your meetings.",

    // Safety mode
    "agent.safety.sandboxed": "Sandboxed",
    "agent.safety.sandboxedDescription":
      "Clovy can read your files but only change its own workspace.",
    "agent.safety.unrestricted": "Unrestricted",
    "agent.safety.unrestrictedDescription":
      "Clovy can change any file your account can without asking for tool approval.",
    "agent.safety.menuLabel": "Safety mode",
    "agent.safety.menuTitle": "Choose what Clovy can touch",
    "agent.safety.triggerTitle": "Change what Clovy can touch",
    "agent.safety.confirmTitle": "Turn on unrestricted?",
    "agent.safety.confirmDescription":
      "Clovy will be able to change any file your account can without asking for tool approval. This comes with risks like data loss if something goes wrong.",
    "agent.safety.confirmAction": "Turn on unrestricted",

    // Workspace
    "agent.autoModel.description": "Chooses the best available model for each request.",
    "agent.artifacts.rootLabel": "Clovy workspace",
    "agent.workspace.homeLabel": "Home",
    "agent.workspace.sessionLabel": "Session",
    "agent.workspace.taskDetails": "Agent task details",
    "agent.home.conversationLabel": "Home conversation",
    "agent.home.suggestions": "Suggestions",
    "agent.home.handoffInterrupted": "Session creation was interrupted. Try again.",
    "agent.home.error.starting": "Wait for Home to finish starting, then send again.",
    "agent.error.waitForAttachments": "Wait for files to finish attaching, then send again.",
    "agent.error.conversationDeleted": "This conversation was deleted. Your message was not sent.",
    "agent.error.createdConversationDeleted":
      "The new conversation was deleted before it could be opened.",
    "agent.error.dropBusy":
      "Wait for the current files to finish attaching, then drop these files again.",
    "agent.error.tooManyFiles": "You can attach up to 8 files at a time.",

    // Queued follow-ups and unsent messages
    "agent.queued.attachments": {
      one: "{count} attachment queued for next turn",
      other: "{count} attachments queued for next turn",
    },
    "agent.queued.withAttachments": "{status}. {attachments}",
    "agent.queued.attachmentsFailed": "Couldn't send queued attachments",
    "agent.queued.followUpFailed": "Couldn't send queued follow-up",
    "agent.queued.steering": "Steering active run",
    "agent.queued.sending": "Sending to active run",
    "agent.queued.followUp": "Queued follow-up",
    "agent.queued.retryAttachments": "Retry queued attachments",
    "agent.queued.retryFollowUp": "Retry queued follow-up",
    "agent.queued.removeAttachments": "Remove queued attachments",
    "agent.queued.removeFollowUp": "Remove queued follow-up",
    "agent.unsent.label": "Unsent message",
    "agent.unsent.retry": "Retry unsent message",
    "agent.unsent.discard": "Discard unsent message",

    // Usage panel
    "agent.usage.title": "Usage",
    "agent.usage.close": "Close usage",
    "agent.usage.model": "Model",
    "agent.usage.provider": "Provider",
    "agent.usage.privacy": "Privacy",
    "agent.usage.route": "Route",
    "agent.usage.reasoningEffort": "Reasoning effort",
    "agent.usage.input": "Input",
    "agent.usage.output": "Output",
    "agent.usage.total": "Total",
    "agent.usage.noTokenCounts": "Token counts were not reported for this request.",
    "agent.usage.latestContext": "Latest request context",
    "agent.usage.contextValue": "{used} of {limit} ({percent}%)",
    "agent.usage.contextUsed": "Context used",

    "agent.usage.tools": "Tools",
    "agent.usage.toolCalls": { one: "{count} call", other: "{count} calls" },
    "agent.usage.toolCallsWithFailures": {
      one: "{count} call, {failures} failed",
      other: "{count} calls, {failures} failed",
    },
    "agent.usage.empty": "No usage reported for this session yet.",

    // Compact context
    "agent.compact.title": "Compact context?",
    "agent.compact.description":
      "Clovy will replace older conversation turns with one visible summary and keep recent turns unchanged.",
    "agent.compact.compacting": "Compacting",
    "agent.compact.action": "Compact context",
    "agent.compact.done": {
      one: "Context compacted. {count} earlier items were replaced with a summary.",
      other: "Context compacted. {count} earlier items were replaced with a summary.",
    },
    "agent.compact.nothing": "There is not enough earlier context to compact yet.",

    // Composer
    "agent.composer.heroPlaceholder": "Ask Clovy anything, run / commands",
    "agent.composer.placeholder": "Send a message",
    "agent.composer.addFilesOrNotes": "Add files or notes",
    "agent.composer.dictate": "Dictate",
    "agent.composer.startDictation": "Start dictation",
    "agent.composer.steer": "Queue next turn",
    "agent.composer.waitForAttachments": "Wait for files to finish attaching",
    "agent.composer.stop": "Stop Clovy",
    "agent.composer.send": "Send message",
    "agent.composer.attachFiles": "Attach files",
    "agent.composer.referenceNote": "Reference a note",
    "agent.composer.scrollToLatest": "Scroll to latest",

    // Thinking indicator
    "agent.thinking.typing": "Clovy is typing",
    "agent.thinking.label": "Thinking…",

    // Approval trays
    "agent.computerUse.label": "Computer use approvals",
    "agent.computerUse.headingApp": "Clovy wants to use {app}",
    "agent.computerUse.headingApps": {
      one: "Clovy wants to use {count} apps",
      other: "Clovy wants to use {count} apps",
    },
    "agent.computerUse.headingUnknown": "Clovy wants to use an app",
    "agent.computerUse.stop": "Stop",
    "agent.computerUse.expand": "Expand Computer use approvals",
    "agent.computerUse.collapse": "Collapse Computer use approvals",
    "agent.computerUse.captureAlt": "Current {app} window before the proposed action",
    "agent.computerUse.expiresAt": "Expires at {time}",
    "agent.computerUse.deny": "Deny",
    "agent.computerUse.allow": "Allow for this task",

    // Rename dialog
    "agent.rename.title": "Rename session",
    "agent.rename.nameLabel": "Name",
    "agent.rename.nameAria": "Session name",

    // Sessions list
    "agent.sessions.title": "Sessions",
    "agent.sessions.new": "New session",
    "agent.sessions.filter.active": "Active",
    "agent.sessions.filter.archived": "Archived",
    "agent.sessions.filter.all": "All",
    "agent.sessions.filterLabel": "Filter sessions by status",
    "agent.sessions.empty.label": "Start your first session",
    "agent.sessions.empty.title": "Put Clovy to work",
    "agent.sessions.empty.description":
      "Ask Clovy to check on your computer, dig through your files, or research a topic. Each session keeps one task's conversation and everything it produces in one place.",
    "agent.sessions.noMatch": "No sessions match “{query}”.",
    "agent.sessions.noArchived": "No archived sessions yet.",
    "agent.sessions.noSessions": "No sessions yet.",
    "agent.sessions.noActive": "No active sessions.",
    "agent.sessions.selection": "Selection",
    "agent.sessions.selectedCount": { one: "{count} selected", other: "{count} selected" },
    "agent.sessions.selectAll": "Select all",
    "agent.sessions.deselectAll": "Deselect all",
    "agent.sessions.move": "Move",
    "agent.sessions.clearSelection": "Clear selection",
    "agent.sessions.bulkDeleteTitle": {
      one: "Delete {count} session?",
      other: "Delete {count} sessions?",
    },
    "agent.sessions.bulkDeleteDescription":
      "This cannot be undone. These agent sessions will be removed.",
    "agent.sessions.deleteSessions": { one: "Delete session", other: "Delete sessions" },
    "agent.sessions.status.needsYou": "Needs you",
    "agent.sessions.status.working": "Working",
    "agent.sessions.untitled": "Untitled session",
    "agent.sessions.preview.routine": "Imported routine history",
    "agent.sessions.preview.conversation": "Conversation",
    "agent.sessions.archivedMeta": "Archived",
    "agent.sessions.selectRow": "Select {title}",
    "agent.sessions.actionsFor": "Actions for {title}",
    "agent.sessions.changeProject": "Change project",
    "agent.sessions.addToProject": "Add to project",
    "agent.sessions.removeFromProject": "Remove from project",
    "agent.sessions.archive": "Archive",
    "agent.sessions.unarchive": "Unarchive",
    "agent.sessions.deleteTitle": 'Delete "{title}"?',
    "agent.sessions.deleteDescription": "This agent session cannot be restored.",
    "agent.sessions.deleteSession": "Delete session",
  },
  "pt-BR": {
    // Hero
    "agent.hero.greeting1": "O que o Clovy pode fazer por você?",
    "agent.hero.greeting2": "No que vamos trabalhar?",
    "agent.hero.greeting3": "Por onde o Clovy deve começar?",
    "agent.hero.greeting4": "O que o Clovy pode tirar das suas mãos?",
    "agent.shortcut.recapNotes.title": "Resumir minhas notas",
    "agent.shortcut.recapNotes.description":
      "O que aconteceu, o que foi decidido e o que ainda está em aberto.",
    "agent.shortcut.research.title": "Pesquisar um assunto",
    "agent.shortcut.research.description":
      "Receba um resumo curto e com fontes sobre qualquer coisa.",
    "agent.shortcut.summarizeFile.title": "Resumir um arquivo",
    "agent.shortcut.summarizeFile.description": "Escolha um documento e veja os pontos principais.",
    "agent.shortcut.healthCheck.title": "Verificar a saúde do meu Mac",
    "agent.shortcut.healthCheck.description":
      "Disco, memória e itens de início de sessão que precisam de atenção.",
    "agent.shortcut.draftFollowUp.title": "Redigir um follow-up",
    "agent.shortcut.draftFollowUp.description":
      "Transforme sua nota de reunião mais recente em uma mensagem de follow-up.",
    "agent.shortcut.findFile.title": "Encontrar um arquivo",
    "agent.shortcut.findFile.description": "Descreva o que você lembra e o Clovy encontra.",
    "agent.shortcut.analyzeSpreadsheet.title": "Analisar uma planilha",
    "agent.shortcut.analyzeSpreadsheet.description":
      "Números principais, tendências e anomalias de um CSV ou planilha.",
    "agent.shortcut.searchNotes.title": "Buscar nas minhas notas",
    "agent.shortcut.searchNotes.description":
      "Descubra onde algo foi mencionado nas suas reuniões.",

    // Safety mode
    "agent.safety.sandboxed": "Em sandbox",
    "agent.safety.sandboxedDescription":
      "O Clovy pode ler seus arquivos, mas só altera o próprio espaço de trabalho.",
    "agent.safety.unrestricted": "Sem restrições",
    "agent.safety.unrestrictedDescription":
      "O Clovy pode alterar qualquer arquivo que sua conta pode alterar, sem pedir aprovação de ferramentas.",
    "agent.safety.menuLabel": "Modo de segurança",
    "agent.safety.menuTitle": "Escolha o que o Clovy pode alterar",
    "agent.safety.triggerTitle": "Mudar o que o Clovy pode alterar",
    "agent.safety.confirmTitle": "Ativar o modo sem restrições?",
    "agent.safety.confirmDescription":
      "O Clovy poderá alterar qualquer arquivo que sua conta pode alterar, sem pedir aprovação de ferramentas. Isso traz riscos, como perda de dados se algo der errado.",
    "agent.safety.confirmAction": "Ativar sem restrições",

    // Workspace
    "agent.autoModel.description": "Escolhe o melhor modelo disponível para cada solicitação.",
    "agent.artifacts.rootLabel": "Espaço de trabalho do Clovy",
    "agent.workspace.homeLabel": "Início",
    "agent.workspace.sessionLabel": "Sessão",
    "agent.workspace.taskDetails": "Detalhes da tarefa do agente",
    "agent.home.conversationLabel": "Conversa do Início",
    "agent.home.suggestions": "Sugestões",
    "agent.home.handoffInterrupted": "A criação da sessão foi interrompida. Tente de novo.",
    "agent.home.error.starting": "Aguarde o Início terminar de carregar e envie de novo.",
    "agent.error.waitForAttachments": "Aguarde os arquivos terminarem de anexar e envie de novo.",
    "agent.error.conversationDeleted": "Esta conversa foi excluída. Sua mensagem não foi enviada.",
    "agent.error.createdConversationDeleted":
      "A nova conversa foi excluída antes de poder ser aberta.",
    "agent.error.dropBusy":
      "Aguarde os arquivos atuais terminarem de anexar e solte estes arquivos de novo.",
    "agent.error.tooManyFiles": "Você pode anexar até 8 arquivos por vez.",

    // Queued follow-ups and unsent messages
    "agent.queued.attachments": {
      one: "{count} anexo na fila para a próxima mensagem",
      other: "{count} anexos na fila para a próxima mensagem",
    },
    "agent.queued.withAttachments": "{status}. {attachments}",
    "agent.queued.attachmentsFailed": "Não foi possível enviar os anexos na fila",
    "agent.queued.followUpFailed": "Não foi possível enviar a mensagem na fila",
    "agent.queued.steering": "Direcionando a execução em andamento",
    "agent.queued.sending": "Enviando para a execução em andamento",
    "agent.queued.followUp": "Mensagem na fila",
    "agent.queued.retryAttachments": "Tentar enviar os anexos na fila de novo",
    "agent.queued.retryFollowUp": "Tentar enviar a mensagem na fila de novo",
    "agent.queued.removeAttachments": "Remover anexos da fila",
    "agent.queued.removeFollowUp": "Remover mensagem da fila",
    "agent.unsent.label": "Mensagem não enviada",
    "agent.unsent.retry": "Tentar enviar a mensagem de novo",
    "agent.unsent.discard": "Descartar mensagem não enviada",

    // Usage panel
    "agent.usage.title": "Uso",
    "agent.usage.close": "Fechar uso",
    "agent.usage.model": "Modelo",
    "agent.usage.provider": "Provedor",
    "agent.usage.privacy": "Privacidade",
    "agent.usage.route": "Rota",
    "agent.usage.reasoningEffort": "Esforço de raciocínio",
    "agent.usage.input": "Entrada",
    "agent.usage.output": "Saída",
    "agent.usage.total": "Total",
    "agent.usage.noTokenCounts": "A contagem de tokens não foi informada para esta solicitação.",
    "agent.usage.latestContext": "Contexto da última solicitação",
    "agent.usage.contextValue": "{used} de {limit} ({percent}%)",
    "agent.usage.contextUsed": "Contexto usado",

    "agent.usage.tools": "Ferramentas",
    "agent.usage.toolCalls": { one: "{count} chamada", other: "{count} chamadas" },
    "agent.usage.toolCallsWithFailures": {
      one: "{count} chamada, {failures} com falha",
      other: "{count} chamadas, {failures} com falha",
    },
    "agent.usage.empty": "Nenhum uso informado para esta sessão ainda.",

    // Compact context
    "agent.compact.title": "Compactar contexto?",
    "agent.compact.description":
      "O Clovy vai substituir as mensagens mais antigas da conversa por um resumo visível e manter as recentes sem alterações.",
    "agent.compact.compacting": "Compactando",
    "agent.compact.action": "Compactar contexto",
    "agent.compact.done": {
      one: "Contexto compactado. {count} item anterior foi substituído por um resumo.",
      other: "Contexto compactado. {count} itens anteriores foram substituídos por um resumo.",
    },
    "agent.compact.nothing": "Ainda não há contexto anterior suficiente para compactar.",

    // Composer
    "agent.composer.heroPlaceholder": "Pergunte qualquer coisa ao Clovy, use / para comandos",
    "agent.composer.placeholder": "Envie uma mensagem",
    "agent.composer.addFilesOrNotes": "Adicionar arquivos ou notas",
    "agent.composer.dictate": "Ditar",
    "agent.composer.startDictation": "Iniciar ditado",
    "agent.composer.steer": "Adicionar próximo turno à fila",
    "agent.composer.waitForAttachments": "Aguarde os arquivos terminarem de anexar",
    "agent.composer.stop": "Parar o Clovy",
    "agent.composer.send": "Enviar mensagem",
    "agent.composer.attachFiles": "Anexar arquivos",
    "agent.composer.referenceNote": "Mencionar uma nota",
    "agent.composer.scrollToLatest": "Rolar até o fim",

    // Thinking indicator
    "agent.thinking.typing": "O Clovy está digitando",
    "agent.thinking.label": "Pensando…",

    // Approval trays
    "agent.computerUse.label": "Aprovações de uso do computador",
    "agent.computerUse.headingApp": "O Clovy quer usar o {app}",
    "agent.computerUse.headingApps": {
      one: "O Clovy quer usar {count} apps",
      other: "O Clovy quer usar {count} apps",
    },
    "agent.computerUse.headingUnknown": "O Clovy quer usar um app",
    "agent.computerUse.stop": "Parar",
    "agent.computerUse.expand": "Expandir aprovações de uso do computador",
    "agent.computerUse.collapse": "Recolher aprovações de uso do computador",
    "agent.computerUse.captureAlt": "Janela atual do {app} antes da ação proposta",
    "agent.computerUse.expiresAt": "Expira às {time}",
    "agent.computerUse.deny": "Negar",
    "agent.computerUse.allow": "Permitir nesta tarefa",

    // Rename dialog
    "agent.rename.title": "Renomear sessão",
    "agent.rename.nameLabel": "Nome",
    "agent.rename.nameAria": "Nome da sessão",

    // Sessions list
    "agent.sessions.title": "Sessões",
    "agent.sessions.new": "Nova sessão",
    "agent.sessions.filter.active": "Ativas",
    "agent.sessions.filter.archived": "Arquivadas",
    "agent.sessions.filter.all": "Todas",
    "agent.sessions.filterLabel": "Filtrar sessões por status",
    "agent.sessions.empty.label": "Comece sua primeira sessão",
    "agent.sessions.empty.title": "Coloque o Clovy para trabalhar",
    "agent.sessions.empty.description":
      "Peça ao Clovy para verificar seu computador, vasculhar seus arquivos ou pesquisar um assunto. Cada sessão reúne em um só lugar a conversa de uma tarefa e tudo o que ela produz.",
    "agent.sessions.noMatch": "Nenhuma sessão corresponde a “{query}”.",
    "agent.sessions.noArchived": "Nenhuma sessão arquivada ainda.",
    "agent.sessions.noSessions": "Nenhuma sessão ainda.",
    "agent.sessions.noActive": "Nenhuma sessão ativa.",
    "agent.sessions.selection": "Seleção",
    "agent.sessions.selectedCount": { one: "{count} selecionada", other: "{count} selecionadas" },
    "agent.sessions.selectAll": "Selecionar tudo",
    "agent.sessions.deselectAll": "Desmarcar tudo",
    "agent.sessions.move": "Mover",
    "agent.sessions.clearSelection": "Limpar seleção",
    "agent.sessions.bulkDeleteTitle": {
      one: "Excluir {count} sessão?",
      other: "Excluir {count} sessões?",
    },
    "agent.sessions.bulkDeleteDescription":
      "Isso não pode ser desfeito. Estas sessões do agente serão removidas.",
    "agent.sessions.deleteSessions": { one: "Excluir sessão", other: "Excluir sessões" },
    "agent.sessions.status.needsYou": "Precisa de você",
    "agent.sessions.status.working": "Trabalhando",
    "agent.sessions.untitled": "Sessão sem título",
    "agent.sessions.preview.routine": "Histórico de rotina importado",
    "agent.sessions.preview.conversation": "Conversa",
    "agent.sessions.archivedMeta": "Arquivada",
    "agent.sessions.selectRow": "Selecionar {title}",
    "agent.sessions.actionsFor": "Ações para {title}",
    "agent.sessions.changeProject": "Mudar de projeto",
    "agent.sessions.addToProject": "Adicionar ao projeto",
    "agent.sessions.removeFromProject": "Remover do projeto",
    "agent.sessions.archive": "Arquivar",
    "agent.sessions.unarchive": "Desarquivar",
    "agent.sessions.deleteTitle": 'Excluir "{title}"?',
    "agent.sessions.deleteDescription": "Esta sessão do agente não poderá ser restaurada.",
    "agent.sessions.deleteSession": "Excluir sessão",
  },
});
