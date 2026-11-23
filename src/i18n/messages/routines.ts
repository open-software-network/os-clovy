import { defineMessages } from "../define";

export default defineMessages({
  en: {
    // Shared labels
    "routines.title": "Routines",
    "routines.newRoutine": "New routine",
    "routines.backToRoutines": "Back to routines",
    "routines.routineName": "Routine name",
    "routines.deleteRoutine": "Delete routine",
    "routines.runNow": "Run now",
    "routines.runHistory": "Run history",
    "routines.paused": "Paused",
    "routines.completed": "Completed",
    "routines.lastRan": "Last ran {time}",
    "routines.unrestrictedTip":
      "This routine runs with full access: when it fires, Clovy can run commands and change any file your account can. Routines without this badge run sandboxed and cannot touch your files.",
    "routines.policyLoading": "Connector policy is still loading. Try again.",
    "routines.googleAccountRequired": "Google account required",
    "routines.section.when": "When",
    "routines.section.instructions": "Instructions",
    "routines.section.access": "Access",
    "routines.section.actions": "Actions",

    // Sandbox mode
    "routines.mode.sandboxed": "Sandboxed",
    "routines.mode.unrestricted": "Unrestricted",
    "routines.mode.question": "What can this routine change?",
    "routines.mode.unrestrictedHint":
      "When it fires, Clovy can run commands and change any file your account can.",
    "routines.mode.sandboxedHint":
      "The routine can read the web, use memory, and message you. It cannot run commands or change your files.",
    "routines.mode.sandboxedOption":
      "The routine can read the web and memory but cannot touch your files.",
    "routines.mode.unrestrictedOption":
      "When it fires, Clovy can change any file your account can.",

    // Run list
    "routines.run.runningNow": "Running now",
    "routines.run.running": "Running",
    "routines.run.today": "today {time}",
    "routines.run.tomorrow": "tomorrow {time}",
    "routines.run.yesterday": "yesterday {time}",
    "routines.run.fallbackLabel": "Routine run",
    "routines.run.result": "Run result",
    "routines.run.loadingResult": "Loading result…",
    "routines.run.unavailableResult": "This run result is unavailable.",
    "routines.run.noResult": "This run has no recorded output.",
    "routines.run.waiting": "Waiting for input",
    "routines.run.finished": "Completed",
    "routines.run.failed": "Failed",
    "routines.run.interrupted": "Interrupted",
    "routines.run.idle": "Idle",

    // Schedule picker
    "routines.schedule.daily": "Daily",
    "routines.schedule.weekdays": "Weekdays",
    "routines.schedule.weekly": "Weekly",
    "routines.schedule.interval": "Interval",
    "routines.schedule.custom": "Custom",
    "routines.schedule.minutes": "minutes",
    "routines.schedule.hours": "hours",
    "routines.schedule.placeholder": "Schedule",
    "routines.schedule.typeLabel": "Schedule type",
    "routines.schedule.day": "Day",
    "routines.schedule.dayOfWeek": "Day of week",
    "routines.schedule.time": "Time",
    "routines.schedule.repeatEvery": "Repeat every",
    "routines.schedule.unit": "Unit",
    "routines.schedule.intervalUnit": "Interval unit",
    "routines.schedule.customLabel": "Custom schedule",
    "routines.schedule.customPlaceholder": "0 9 * * 1-5 or every 30m",
    "routines.schedule.customHelp":
      'A cron expression, an interval like "every 30m", or a date for a one-time run.',

    // Trigger picker
    "routines.trigger.onSchedule": "On a schedule",
    "routines.trigger.placeholder": "When",
    "routines.trigger.typeLabel": "Trigger type",
    "routines.trigger.leadLabel": "Minutes before the meeting",
    "routines.trigger.minutesBefore": "minutes before",
    "routines.trigger.externalOnly": "Only meetings with external guests",
    "routines.trigger.needsAccount":
      "Event triggers need a connected Google account. Connect one in Settings under Plugins.",
    "routines.trigger.moreAccess": "More Google access needed",

    // Trust picker
    "routines.trust.question": "What can this routine do with your Google account?",
    "routines.trust.grantsLegend": "Tools this routine may run unasked",
    "routines.trust.summary": "Trust: {trust}.",
    "routines.trust.toolSummary": "{summary}. Trust: {trust}.",

    // Create
    "routines.create.notConfigured": "Google connector isn't configured in this build.",
    "routines.create.creating": "Creating…",
    "routines.create.needsAccount":
      "This routine needs a connected Google account with the listed access before it can be created.",
    "routines.create.waitingForBrowser": "Waiting for browser…",
    "routines.create.connectGoogle": "Connect Google account",
    "routines.create.instructionsPlaceholder":
      "Summarize my unread notes and list anything that needs a reply…",

    // Detail
    "routines.detail.connectGoogleFirst": "Connect a Google account before using an event trigger.",
    "routines.detail.restoreFailed": "Clovy could not restore the previous inactive state: {error}",
    "routines.detail.actions": "Routine actions",
    "routines.detail.queued": "Queued",
    "routines.detail.saving": "Saving…",
    "routines.detail.activeToggle": "{name} active",
    "routines.detail.active": "Active",
    "routines.detail.lastRunFailed": "Last run failed.",
    "routines.detail.sections": "Routine sections",
    "routines.detail.details": "Details",
    "routines.detail.browserUse": "Browser use",
    "routines.detail.browserUseDescription":
      "Allow this routine to read and navigate public pages with a separate anonymous profile. Saved logins and interaction with page controls are unavailable.",
    "routines.detail.browserUseToggle": "Allow browser use for this routine",
    "routines.detail.scriptNote":
      "This routine has an attached script ({script}) that runs outside the sandbox, so it always has full access. Switching it to Sandboxed removes the script when you save.",
    "routines.detail.noRuns": "No runs yet. When this routine runs, its result appears here.",

    // List view
    "routines.view.cleanupFailed":
      "{error} Clovy also could not remove the partially created routine: {cleanupError}",
    "routines.view.deleteTitle": "Delete “{name}”?",
    "routines.view.deleteDescription":
      "Clovy will stop running this routine. This can’t be undone.",
    "routines.view.subtitle": "Automations Clovy runs for you on a schedule.",
    "routines.view.refresh": "Refresh",
    "routines.view.loading": "Loading routines…",
    "routines.view.noMatch": "No routines match “{query}”.",
    "routines.view.runsUnavailable": "Run history is unavailable right now.",
    "routines.view.noRuns": "No runs yet. When a routine runs, its result appears here.",
    "routines.view.starters": "Starter routines",
    "routines.view.starterUnrestrictedTip":
      "This starter needs full access: when it fires, Clovy can run commands and change any file your account can. You confirm that before creating it.",
    "routines.view.addTemplate": "Add {name}",
    "routines.view.lastRunFailed": "Last run failed",
    "routines.view.actionsFor": "Actions for {name}",
    "routines.view.serverError": "Clovy ran into a problem with that request.",
    "routines.view.unavailable": "Routines are unavailable. Try again.",

    // Starter templates
    "routines.template.morningBrief.name": "Morning brief",
    "routines.template.morningBrief.description":
      "Open loops from recent sessions, your todos, and anything new that matters.",
    "routines.template.weeklyReview.name": "Weekly review",
    "routines.template.weeklyReview.description":
      "A Friday afternoon summary of the week's work and what to carry forward.",
    "routines.template.newsWatch.name": "News watch",
    "routines.template.newsWatch.description":
      "Track a topic and only hear about it when something actually happens.",
    "routines.template.memoryTidy.name": "Memory tidy",
    "routines.template.memoryTidy.description":
      "A weekly pass over Clovy's memory to merge duplicates and drop stale facts.",
    "routines.template.morningBriefing.name": "Morning briefing",
    "routines.template.morningBriefing.description":
      "Today's calendar, a summary of unread mail, and prep for what's ahead.",
    "routines.template.autoInbox.name": "Auto-inbox",
    "routines.template.autoInbox.description":
      "Triage new mail as it arrives, label it, and draft replies for your approval.",
    "routines.template.meetingPrep.name": "Meeting prep",
    "routines.template.meetingPrep.description":
      "A brief before meetings with external guests: who they are and where you left off.",
    "routines.template.downloadsTidy.name": "Tidy downloads",
    "routines.template.downloadsTidy.description":
      "Sort the Downloads folder into subfolders by type every Friday.",
    "routines.template.tools.mailCalendar": "This routine can: read your mail, read your calendar",
    "routines.template.tools.inbox":
      "This routine can: read your mail, draft replies, label and archive",
  },
  "pt-BR": {
    "routines.title": "Rotinas",
    "routines.newRoutine": "Nova rotina",
    "routines.backToRoutines": "Voltar para rotinas",
    "routines.routineName": "Nome da rotina",
    "routines.deleteRoutine": "Excluir rotina",
    "routines.runNow": "Executar agora",
    "routines.runHistory": "Histórico de execuções",
    "routines.paused": "Pausada",
    "routines.completed": "Concluída",
    "routines.lastRan": "Última execução: {time}",
    "routines.unrestrictedTip":
      "Esta rotina roda com acesso total: quando for executada, o Clovy pode rodar comandos e alterar qualquer arquivo que sua conta pode alterar. Rotinas sem este selo rodam em sandbox e não podem mexer nos seus arquivos.",
    "routines.policyLoading": "A política de conectores ainda está carregando. Tente de novo.",
    "routines.googleAccountRequired": "Conta Google necessária",
    "routines.section.when": "Quando",
    "routines.section.instructions": "Instruções",
    "routines.section.access": "Acesso",
    "routines.section.actions": "Ações",

    "routines.mode.sandboxed": "Em sandbox",
    "routines.mode.unrestricted": "Sem restrições",
    "routines.mode.question": "O que esta rotina pode alterar?",
    "routines.mode.unrestrictedHint":
      "Quando for executada, o Clovy pode rodar comandos e alterar qualquer arquivo que sua conta pode alterar.",
    "routines.mode.sandboxedHint":
      "A rotina pode ler a web, usar a memória e enviar mensagens para você. Ela não pode rodar comandos nem alterar seus arquivos.",
    "routines.mode.sandboxedOption":
      "A rotina pode ler a web e a memória, mas não pode mexer nos seus arquivos.",
    "routines.mode.unrestrictedOption":
      "Quando for executada, o Clovy pode alterar qualquer arquivo que sua conta pode alterar.",

    "routines.run.runningNow": "Em execução agora",
    "routines.run.running": "Em execução",
    "routines.run.today": "hoje às {time}",
    "routines.run.tomorrow": "amanhã às {time}",
    "routines.run.yesterday": "ontem às {time}",
    "routines.run.fallbackLabel": "Execução de rotina",
    "routines.run.result": "Resultado da execução",
    "routines.run.loadingResult": "Carregando resultado…",
    "routines.run.unavailableResult": "O resultado desta execução não está disponível.",
    "routines.run.noResult": "Esta execução não tem um resultado registrado.",
    "routines.run.waiting": "Aguardando resposta",
    "routines.run.finished": "Concluída",
    "routines.run.failed": "Falhou",
    "routines.run.interrupted": "Interrompida",
    "routines.run.idle": "Inativa",

    "routines.schedule.daily": "Diariamente",
    "routines.schedule.weekdays": "Dias úteis",
    "routines.schedule.weekly": "Semanalmente",
    "routines.schedule.interval": "Intervalo",
    "routines.schedule.custom": "Personalizado",
    "routines.schedule.minutes": "minutos",
    "routines.schedule.hours": "horas",
    "routines.schedule.placeholder": "Programação",
    "routines.schedule.typeLabel": "Tipo de programação",
    "routines.schedule.day": "Dia",
    "routines.schedule.dayOfWeek": "Dia da semana",
    "routines.schedule.time": "Horário",
    "routines.schedule.repeatEvery": "Repetir a cada",
    "routines.schedule.unit": "Unidade",
    "routines.schedule.intervalUnit": "Unidade do intervalo",
    "routines.schedule.customLabel": "Programação personalizada",
    "routines.schedule.customPlaceholder": "0 9 * * 1-5 ou every 30m",
    "routines.schedule.customHelp":
      'Uma expressão cron, um intervalo como "every 30m" ou uma data para uma execução única.',

    "routines.trigger.onSchedule": "Por programação",
    "routines.trigger.placeholder": "Quando",
    "routines.trigger.typeLabel": "Tipo de gatilho",
    "routines.trigger.leadLabel": "Minutos antes da reunião",
    "routines.trigger.minutesBefore": "minutos antes",
    "routines.trigger.externalOnly": "Somente reuniões com convidados externos",
    "routines.trigger.needsAccount":
      "Gatilhos de evento precisam de uma conta Google conectada. Conecte uma em Configurações, na seção Plugins.",
    "routines.trigger.moreAccess": "É necessário mais acesso ao Google",

    "routines.trust.question": "O que esta rotina pode fazer com sua conta Google?",
    "routines.trust.grantsLegend": "Ferramentas que esta rotina pode usar sem perguntar",
    "routines.trust.summary": "Confiança: {trust}.",
    "routines.trust.toolSummary": "{summary}. Confiança: {trust}.",

    "routines.create.notConfigured": "O conector do Google não está configurado nesta versão.",
    "routines.create.creating": "Criando…",
    "routines.create.needsAccount":
      "Esta rotina precisa de uma conta Google conectada com os acessos listados antes de ser criada.",
    "routines.create.waitingForBrowser": "Aguardando o navegador…",
    "routines.create.connectGoogle": "Conectar conta Google",
    "routines.create.instructionsPlaceholder":
      "Resuma minhas notas não lidas e liste o que precisa de resposta…",

    "routines.detail.connectGoogleFirst":
      "Conecte uma conta Google antes de usar um gatilho de evento.",
    "routines.detail.restoreFailed":
      "O Clovy não conseguiu restaurar o estado inativo anterior: {error}",
    "routines.detail.actions": "Ações da rotina",
    "routines.detail.queued": "Na fila",
    "routines.detail.saving": "Salvando…",
    "routines.detail.activeToggle": "{name} ativa",
    "routines.detail.active": "Ativa",
    "routines.detail.lastRunFailed": "A última execução falhou.",
    "routines.detail.sections": "Seções da rotina",
    "routines.detail.details": "Detalhes",
    "routines.detail.browserUse": "Uso do navegador",
    "routines.detail.browserUseDescription":
      "Permita que esta rotina leia e navegue em páginas públicas com um perfil anônimo separado. Logins salvos e interação com controles da página ficam indisponíveis.",
    "routines.detail.browserUseToggle": "Permitir uso do navegador nesta rotina",
    "routines.detail.scriptNote":
      "Esta rotina tem um script anexado ({script}) que roda fora do sandbox, então sempre tem acesso total. Mudar para Em sandbox remove o script quando você salvar.",
    "routines.detail.noRuns":
      "Nenhuma execução ainda. Quando esta rotina for executada, a sessão aparecerá aqui.",

    "routines.view.cleanupFailed":
      "{error} O Clovy também não conseguiu remover a rotina criada parcialmente: {cleanupError}",
    "routines.view.deleteTitle": "Excluir “{name}”?",
    "routines.view.deleteDescription":
      "O Clovy vai parar de executar esta rotina. Não é possível desfazer.",
    "routines.view.subtitle": "Automações que o Clovy executa para você em horários programados.",
    "routines.view.refresh": "Atualizar",
    "routines.view.loading": "Carregando rotinas…",
    "routines.view.noMatch": "Nenhuma rotina corresponde a “{query}”.",
    "routines.view.runsUnavailable": "O histórico de execuções não está disponível agora.",
    "routines.view.noRuns":
      "Nenhuma execução ainda. Quando uma rotina for executada, a sessão aparecerá aqui.",
    "routines.view.starters": "Rotinas iniciais",
    "routines.view.starterUnrestrictedTip":
      "Esta rotina inicial precisa de acesso total: quando for executada, o Clovy pode rodar comandos e alterar qualquer arquivo que sua conta pode alterar. Você confirma isso antes de criá-la.",
    "routines.view.addTemplate": "Adicionar {name}",
    "routines.view.lastRunFailed": "A última execução falhou",
    "routines.view.actionsFor": "Ações de {name}",
    "routines.view.serverError": "O Clovy encontrou um problema com essa solicitação.",
    "routines.view.unavailable": "As rotinas não estão disponíveis. Tente de novo.",

    "routines.template.morningBrief.name": "Resumo da manhã",
    "routines.template.morningBrief.description":
      "Pendências das sessões recentes, suas tarefas e novidades que importam.",
    "routines.template.weeklyReview.name": "Revisão semanal",
    "routines.template.weeklyReview.description":
      "Um resumo na sexta à tarde do trabalho da semana e do que levar adiante.",
    "routines.template.newsWatch.name": "Monitor de notícias",
    "routines.template.newsWatch.description":
      "Acompanhe um assunto e só receba avisos quando algo realmente acontecer.",
    "routines.template.memoryTidy.name": "Organizar memória",
    "routines.template.memoryTidy.description":
      "Uma revisão semanal da memória do Clovy para unir duplicatas e remover fatos desatualizados.",
    "routines.template.morningBriefing.name": "Briefing matinal",
    "routines.template.morningBriefing.description":
      "A agenda de hoje, um resumo dos e-mails não lidos e a preparação para o que vem pela frente.",
    "routines.template.autoInbox.name": "Caixa de entrada automática",
    "routines.template.autoInbox.description":
      "Faça a triagem dos e-mails assim que chegam, aplique marcadores e rascunhe respostas para sua aprovação.",
    "routines.template.meetingPrep.name": "Preparação para reuniões",
    "routines.template.meetingPrep.description":
      "Um resumo antes de reuniões com convidados externos: quem são e onde vocês pararam.",
    "routines.template.downloadsTidy.name": "Organizar downloads",
    "routines.template.downloadsTidy.description":
      "Organize a pasta Downloads em subpastas por tipo toda sexta-feira.",
    "routines.template.tools.mailCalendar": "Esta rotina pode: ler seus e-mails, ler sua agenda",
    "routines.template.tools.inbox":
      "Esta rotina pode: ler seus e-mails, rascunhar respostas, aplicar marcadores e arquivar",
  },
});
