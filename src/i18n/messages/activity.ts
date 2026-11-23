import { defineMessages } from "../define";

export default defineMessages({
  en: {
    "activity.tab.label": "Activity",
    "activity.title": "Activity",
    "activity.blurb":
      "Text-only capture from your active apps, stored in an encrypted local database. Nothing leaves this device, and no screenshots are saved.",

    "activity.capture.title": "Capture activity",
    "activity.capture.description":
      "Record window titles, visible text, and input activity for your timeline.",

    "activity.status.active": "Active",
    "activity.status.off": "Capture is turned off",
    "activity.status.pausedManual": "Paused (manual)",
    "activity.status.pausedWorkHours": "Paused (outside work hours)",
    "activity.status.pausedLowDisk": "Paused (low disk space)",
    "activity.status.pausedProtectedVideo": "Paused (protected video playing)",
    "activity.status.needsPermissions": "Needs permissions: {missing}",
    "activity.status.keyMissing": "Encryption key missing from Keychain",
    "activity.status.error": "Error: {message}",
    "activity.status.lastFrameAt": "Last frame captured at {time}",

    "activity.permissions.title": "Permissions",
    "activity.permissions.description":
      "Manage system permissions required to capture activity and input.",

    "activity.permissions.accessibility.title": "Accessibility",
    "activity.permissions.accessibility.description":
      "Required to inspect window titles and active UI elements.",

    "activity.permissions.screenRecording.title": "Screen recording",
    "activity.permissions.screenRecording.description":
      "Required to read visible on-screen text. No screenshots or recordings are saved to disk.",

    "activity.permissions.inputMonitoring.title": "Input monitoring",
    "activity.permissions.inputMonitoring.description":
      "Optional. Used to count clicks and keystrokes without logging what was typed.",

    "activity.permissions.status.granted": "Granted",
    "activity.permissions.status.missing": "Missing",
    "activity.permissions.status.optionalMissing": "Not granted",

    "activity.permissions.request": "Request",
    "activity.permissions.requestAria": "Request {permission} permission",
    "activity.permissions.openSettings": "Open System Settings",
    "activity.permissions.openSettingsAria": "Open System Settings for {permission}",

    "activity.controls.pause": "Pause capture",
    "activity.controls.resume": "Resume capture",

    "activity.keyMissing.title": "Encryption key missing",
    "activity.keyMissing.description":
      "The encryption key for your activity database is no longer in the Keychain. Old activity data cannot be read. You can recreate the database to start fresh.",
    "activity.keyMissing.recreate": "Recreate database",
    "activity.keyMissing.confirmTitle": "Recreate activity database",
    "activity.keyMissing.confirmDescription":
      "Recreating the database permanently erases existing unreadable activity data. Are you sure you want to proceed?",
    "activity.keyMissing.confirmAction": "Recreate",
    "activity.keyMissing.cancelAction": "Cancel",

    "activity.exclusions.title": "Exclusions",
    "activity.exclusions.description":
      "Private or incognito windows and Clovy windows are always excluded.",

    "activity.exclusions.apps.title": "Ignored applications",
    "activity.exclusions.apps.description":
      "Applications that will never be captured or inspected.",
    "activity.exclusions.apps.placeholder": "Application name (e.g. 1Password)",
    "activity.exclusions.apps.add": "Add",
    "activity.exclusions.apps.addAria": "Add application",
    "activity.exclusions.apps.removeAria": "Remove {name} from ignored applications",
    "activity.exclusions.apps.empty": "No applications excluded",

    "activity.exclusions.domains.title": "Ignored domains",
    "activity.exclusions.domains.description":
      "Domains that will never be captured. Subdomains are automatically included.",
    "activity.exclusions.domains.placeholder": "Domain (e.g. youtube.com)",
    "activity.exclusions.domains.add": "Add",
    "activity.exclusions.domains.addAria": "Add domain",
    "activity.exclusions.domains.removeAria": "Remove {domain} from ignored domains",
    "activity.exclusions.domains.empty": "No domains excluded",

    "activity.workHours.title": "Work hours",
    "activity.workHours.enable": "Limit capture to work hours",
    "activity.workHours.enableDescription":
      "Automatically pause capture outside scheduled work days and hours.",
    "activity.workHours.daysTitle": "Active days",
    "activity.workHours.day.1": "Mon",
    "activity.workHours.day.2": "Tue",
    "activity.workHours.day.3": "Wed",
    "activity.workHours.day.4": "Thu",
    "activity.workHours.day.5": "Fri",
    "activity.workHours.day.6": "Sat",
    "activity.workHours.day.7": "Sun",
    "activity.workHours.start": "Start time",
    "activity.workHours.end": "End time",

    "activity.retention.title": "Retention",
    "activity.retention.days": "Retention days",
    "activity.retention.daysDescription":
      "Days to keep captured activity (1 to 365). Only data already processed into the timeline is deleted.",

    "activity.options.title": "Capture options",
    "activity.options.secondaryMonitors": "Secondary monitors",
    "activity.options.secondaryMonitorsDescription":
      "Sample text from additional connected displays approximately every 10 seconds.",
    "activity.options.pauseOnProtectedVideo": "Pause on protected video",
    "activity.options.pauseOnProtectedVideoDescription":
      "Pause capture while a streaming app or site with protected video, like Netflix, is in front.",
    "activity.options.inputEvents": "Input events",
    "activity.options.inputEventsDescription":
      "Record click and keystroke counts using Input monitoring.",

    "activity.debug.title": "Diagnostics",
    "activity.debug.export": "Debug export",
    "activity.debug.exportDescription": "Export raw activity capture statistics for diagnostics.",
    "activity.debug.exportButton": "Export diagnostic data",
    "activity.debug.exportResult":
      "Exported to {path} ({frames} frames, {secondaryFrames} secondary frames, {inputEvents} input events, {pauses} pauses)",

    // Activity timeline & today view
    "activity.category.coding": "Coding",
    "activity.category.codeReview": "Code review",
    "activity.category.meeting": "Meeting",
    "activity.category.communication": "Communication",
    "activity.category.design": "Design",
    "activity.category.documentation": "Documentation",
    "activity.category.planning": "Planning",
    "activity.category.deploymentDevops": "Deploy and DevOps",
    "activity.category.research": "Research",
    "activity.category.idlePersonal": "Idle or personal",

    "activity.gap.idle": "Idle",
    "activity.gap.sleep": "System sleep",
    "activity.gap.pausedManual": "Paused (manual)",
    "activity.gap.pausedWorkHours": "Paused (outside work hours)",
    "activity.gap.pausedLowDisk": "Paused (low disk space)",
    "activity.gap.pausedProtectedVideo": "Paused (protected video)",

    "activity.timeline.title": "Today",
    "activity.timeline.prevDay": "Previous day",
    "activity.timeline.nextDay": "Next day",
    "activity.timeline.jumpToday": "Today",
    "activity.timeline.live": "Live",
    "activity.timeline.emptyDay": "No activity recorded for this day.",
    "activity.timeline.capturePausedHint": "Activity capture is currently off.",

    "activity.stats.focused": "Focused",
    "activity.stats.idle": "Idle",
    "activity.stats.away": "Away",
    "activity.stats.topApps": "Top apps",
    "activity.stats.categories": "Categories",

    "activity.search.placeholder": "Search activity, apps, and window titles",
    "activity.search.today": "Today",
    "activity.search.last7Days": "Last 7 days",
    "activity.search.last30Days": "Last 30 days",
    "activity.search.all": "All",
    "activity.search.noResults": 'No results found for "{query}"',
    "activity.search.searching": "Searching\u2026",
    "activity.search.count": { one: "{count} result", other: "{count} results" },
    "activity.search.failed": "Search failed",
    "activity.search.clear": "Clear search",

    "activity.detail.windows": "Windows",
    "activity.detail.excerpt": "Text excerpt",
    "activity.detail.close": "Close details",
    "activity.detail.openInTimeline": "Open in timeline",
    "activity.detail.frameCount": { one: "{count} frame", other: "{count} frames" },
    "activity.detail.timeRange": "{start} - {end}",

    "activity.emptyState.title": "Activity timeline",
    "activity.emptyState.description":
      "Text-only activity capture records window titles and visible text into an encrypted database stored only on this device. Nothing leaves this device, and no screenshots are saved.",
    "activity.emptyState.turnOn": "Turn on in Settings",
    "activity.unsupported.title": "Activity capture is not supported",
    "activity.unsupported.description": "Activity capture is not supported on this system.",
    "activity.state.keyMissingNotice":
      "Encryption key is missing from Keychain. Recreate the database in Settings to resume capture.",
    "activity.state.errorNotice": "Could not load timeline: {message}",
    "activity.state.unknownError": "Unknown error",
    "activity.state.openSettings": "Open Settings",
    "activity.lanes.sessions": "Activity",

    "activity.view.label": "Show by",
    "activity.view.workstreams": "Workstreams",
    "activity.view.sessions": "Sessions",
    "activity.grid.label": "Workstreams of the day",
    "activity.grid.count": { one: "{count} workstream", other: "{count} workstreams" },
    "activity.grid.active": "{duration} active",
    "activity.grid.sittings": { one: "{count} sitting", other: "{count} sittings" },
    "activity.grid.moreNotes": "+{count} more",
    "activity.grid.byHour": "By hour",
    "activity.grid.byHourHint":
      "Part of this work is known only by its hour: no session was tied to it, so there is no exact time.",
    "activity.grid.now": "Now",
    "activity.grid.noProvider":
      "Workstreams need an activity provider. The captured sessions are under Sessions.",
    "activity.grid.chooseProvider": "Choose a provider",
    "activity.grid.pending":
      "Workstreams appear after each hour ends and is summarized. The captured sessions are under Sessions.",
    "activity.grid.loadError": "Could not load workstreams: {message}",
    "activity.workstream.details": "Workstream details",
    "activity.workstream.kicker": "Workstream",
    "activity.workstream.back": "Back to the day overview",
    "activity.workstream.when": "When",
    "activity.workstream.break": "Break · {duration}",
    "activity.workstream.hourActive": "{duration} active in this hour",
    "activity.workstream.hourUnlocated": "{duration} in this hour without an exact time",
    "activity.workstream.whenHint":
      "Times of the sessions this work was found in. A session can hold more than one workstream, so these are not its duration; the active time above is.",
    "activity.workstream.done": "What was done",
  },
  "pt-BR": {
    "activity.tab.label": "Atividade",
    "activity.title": "Atividade",
    "activity.blurb":
      "Captura apenas de texto dos seus aplicativos ativos, armazenada em um banco de dados local criptografado. Nada sai deste dispositivo e nenhuma captura de tela é salva.",

    "activity.capture.title": "Capturar atividade",
    "activity.capture.description":
      "Grave títulos de janelas, texto visível e atividade de entrada para a sua linha do tempo.",

    "activity.status.active": "Ativo",
    "activity.status.off": "A captura está desativada",
    "activity.status.pausedManual": "Pausado (manual)",
    "activity.status.pausedWorkHours": "Pausado (fora do horário de trabalho)",
    "activity.status.pausedLowDisk": "Pausado (pouco espaço em disco)",
    "activity.status.pausedProtectedVideo": "Pausado (reproduzindo vídeo protegido)",
    "activity.status.needsPermissions": "Permissões necessárias: {missing}",
    "activity.status.keyMissing": "Chave de criptografia ausente nas Chaves do sistema",
    "activity.status.error": "Erro: {message}",
    "activity.status.lastFrameAt": "Último quadro capturado às {time}",

    "activity.permissions.title": "Permissões",
    "activity.permissions.description":
      "Gerencie as permissões do sistema necessárias para capturar atividade e entrada.",

    "activity.permissions.accessibility.title": "Acessibilidade",
    "activity.permissions.accessibility.description":
      "Necessário para inspecionar títulos de janelas e elementos ativos da interface.",

    "activity.permissions.screenRecording.title": "Gravação de tela",
    "activity.permissions.screenRecording.description":
      "Necessário para ler texto visível na tela. Nenhuma captura ou gravação é salva no disco.",

    "activity.permissions.inputMonitoring.title": "Monitoramento de entrada",
    "activity.permissions.inputMonitoring.description":
      "Opcional. Usado para contar cliques e teclas sem registrar o que foi digitado.",

    "activity.permissions.status.granted": "Concedido",
    "activity.permissions.status.missing": "Ausente",
    "activity.permissions.status.optionalMissing": "Não concedido",

    "activity.permissions.request": "Solicitar",
    "activity.permissions.requestAria": "Solicitar permissão de {permission}",
    "activity.permissions.openSettings": "Abrir Ajustes do Sistema",
    "activity.permissions.openSettingsAria": "Abrir Ajustes do Sistema para {permission}",

    "activity.controls.pause": "Pausar captura",
    "activity.controls.resume": "Retomar captura",

    "activity.keyMissing.title": "Chave de criptografia ausente",
    "activity.keyMissing.description":
      "A chave de criptografia do banco de dados de atividade não está mais nas Chaves do sistema. Os dados de atividade anteriores não podem ser lidos. Você pode recriar o banco de dados para reiniciar.",
    "activity.keyMissing.recreate": "Recriar banco de dados",
    "activity.keyMissing.confirmTitle": "Recriar banco de dados de atividade",
    "activity.keyMissing.confirmDescription":
      "Recriar o banco de dados apagará permanentemente os dados de atividade ilegíveis. Tem certeza de que deseja continuar?",
    "activity.keyMissing.confirmAction": "Recriar",
    "activity.keyMissing.cancelAction": "Cancelar",

    "activity.exclusions.title": "Exclusões",
    "activity.exclusions.description":
      "Janelas privadas ou anônimas e janelas do Clovy são sempre excluídas.",

    "activity.exclusions.apps.title": "Aplicativos ignorados",
    "activity.exclusions.apps.description":
      "Aplicativos que nunca serão capturados ou inspecionados.",
    "activity.exclusions.apps.placeholder": "Nome do aplicativo (ex. 1Password)",
    "activity.exclusions.apps.add": "Adicionar",
    "activity.exclusions.apps.addAria": "Adicionar aplicativo",
    "activity.exclusions.apps.removeAria": "Remover {name} dos aplicativos ignorados",
    "activity.exclusions.apps.empty": "Nenhum aplicativo excluído",

    "activity.exclusions.domains.title": "Domínios ignorados",
    "activity.exclusions.domains.description":
      "Domínios que nunca serão capturados. Subdomínios são incluídos automaticamente.",
    "activity.exclusions.domains.placeholder": "Domínio (ex. youtube.com)",
    "activity.exclusions.domains.add": "Adicionar",
    "activity.exclusions.domains.addAria": "Adicionar domínio",
    "activity.exclusions.domains.removeAria": "Remover {domain} dos domínios ignorados",
    "activity.exclusions.domains.empty": "Nenhum domínio excluído",

    "activity.workHours.title": "Horário de trabalho",
    "activity.workHours.enable": "Limitar captura ao horário de trabalho",
    "activity.workHours.enableDescription":
      "Pausar a captura automaticamente fora dos dias e horários de trabalho agendados.",
    "activity.workHours.daysTitle": "Dias ativos",
    "activity.workHours.day.1": "Seg",
    "activity.workHours.day.2": "Ter",
    "activity.workHours.day.3": "Qua",
    "activity.workHours.day.4": "Qui",
    "activity.workHours.day.5": "Sex",
    "activity.workHours.day.6": "Sáb",
    "activity.workHours.day.7": "Dom",
    "activity.workHours.start": "Horário de início",
    "activity.workHours.end": "Horário de término",

    "activity.retention.title": "Retenção",
    "activity.retention.days": "Dias de retenção",
    "activity.retention.daysDescription":
      "Dias para manter a atividade capturada (1 a 365). Apenas dados já processados na linha do tempo são excluídos.",

    "activity.options.title": "Opções de captura",
    "activity.options.secondaryMonitors": "Monitores secundários",
    "activity.options.secondaryMonitorsDescription":
      "Amostra texto de monitores adicionais conectados a cada 10 segundos aproximadamente.",
    "activity.options.pauseOnProtectedVideo": "Pausar em vídeo protegido",
    "activity.options.pauseOnProtectedVideoDescription":
      "Pausar a captura enquanto um app ou site de streaming com vídeo protegido, como a Netflix, estiver em primeiro plano.",
    "activity.options.inputEvents": "Eventos de entrada",
    "activity.options.inputEventsDescription":
      "Registrar contagens de cliques e teclas usando monitoramento de entrada.",

    "activity.debug.title": "Diagnósticos",
    "activity.debug.export": "Exportação de depuração",
    "activity.debug.exportDescription":
      "Exportar estatísticas brutas de captura de atividade para diagnósticos.",
    "activity.debug.exportButton": "Exportar dados de diagnóstico",
    "activity.debug.exportResult":
      "Exportado para {path} ({frames} quadros, {secondaryFrames} quadros secundários, {inputEvents} eventos de entrada, {pauses} pausas)",

    // Activity timeline & today view
    "activity.category.coding": "Programação",
    "activity.category.codeReview": "Revisão de código",
    "activity.category.meeting": "Reunião",
    "activity.category.communication": "Comunicação",
    "activity.category.design": "Design",
    "activity.category.documentation": "Documentação",
    "activity.category.planning": "Planejamento",
    "activity.category.deploymentDevops": "Deploy e DevOps",
    "activity.category.research": "Pesquisa",
    "activity.category.idlePersonal": "Ocioso ou pessoal",

    "activity.gap.idle": "Ocioso",
    "activity.gap.sleep": "Suspensão do sistema",
    "activity.gap.pausedManual": "Pausado (manual)",
    "activity.gap.pausedWorkHours": "Pausado (fora do expediente)",
    "activity.gap.pausedLowDisk": "Pausado (pouco espaço em disco)",
    "activity.gap.pausedProtectedVideo": "Pausado (vídeo protegido)",

    "activity.timeline.title": "Hoje",
    "activity.timeline.prevDay": "Dia anterior",
    "activity.timeline.nextDay": "Próximo dia",
    "activity.timeline.jumpToday": "Hoje",
    "activity.timeline.live": "Ao vivo",
    "activity.timeline.emptyDay": "Nenhuma atividade registrada para este dia.",
    "activity.timeline.capturePausedHint": "A captura de atividade está desativada no momento.",

    "activity.stats.focused": "Focado",
    "activity.stats.idle": "Ocioso",
    "activity.stats.away": "Ausente",
    "activity.stats.topApps": "Principais apps",
    "activity.stats.categories": "Categorias",

    "activity.search.placeholder": "Buscar atividade, apps e títulos de janela",
    "activity.search.today": "Hoje",
    "activity.search.last7Days": "Últimos 7 dias",
    "activity.search.last30Days": "Últimos 30 dias",
    "activity.search.all": "Tudo",
    "activity.search.noResults": 'Nenhum resultado encontrado para "{query}"',
    "activity.search.searching": "Buscando\u2026",
    "activity.search.count": { one: "{count} resultado", other: "{count} resultados" },
    "activity.search.failed": "A busca falhou",
    "activity.search.clear": "Limpar busca",

    "activity.detail.windows": "Janelas",
    "activity.detail.excerpt": "Trecho de texto",
    "activity.detail.close": "Fechar detalhes",
    "activity.detail.openInTimeline": "Abrir na linha do tempo",
    "activity.detail.frameCount": { one: "{count} quadro", other: "{count} quadros" },
    "activity.detail.timeRange": "{start} - {end}",

    "activity.emptyState.title": "Linha do tempo de atividade",
    "activity.emptyState.description":
      "A captura apenas de texto grava títulos de janelas e texto visível em um banco de dados criptografado armazenado apenas neste dispositivo. Nada sai deste dispositivo e nenhuma captura de tela é salva.",
    "activity.emptyState.turnOn": "Ativar nas Configurações",
    "activity.unsupported.title": "Captura de atividade não suportada",
    "activity.unsupported.description": "A captura de atividade não é suportada neste sistema.",
    "activity.state.keyMissingNotice":
      "A chave de criptografia está ausente do Keychain. Recrie o banco nas Configurações para retomar a captura.",
    "activity.state.errorNotice": "Não foi possível carregar a linha do tempo: {message}",
    "activity.state.unknownError": "Erro desconhecido",
    "activity.state.openSettings": "Abrir Configurações",
    "activity.lanes.sessions": "Atividade",

    "activity.view.label": "Mostrar por",
    "activity.view.workstreams": "Frentes",
    "activity.view.sessions": "Sessões",
    "activity.grid.label": "Frentes de trabalho do dia",
    "activity.grid.count": {
      one: "{count} frente de trabalho",
      other: "{count} frentes de trabalho",
    },
    "activity.grid.active": "{duration} ativos",
    "activity.grid.sittings": { one: "{count} período", other: "{count} períodos" },
    "activity.grid.moreNotes": "+{count} mais",
    "activity.grid.byHour": "Por hora",
    "activity.grid.byHourHint":
      "Parte deste trabalho é conhecida só pela hora: nenhuma sessão foi associada a ela, então não há horário exato.",
    "activity.grid.now": "Agora",
    "activity.grid.noProvider":
      "As frentes de trabalho precisam de um provedor de atividade. As sessões capturadas estão em Sessões.",
    "activity.grid.chooseProvider": "Escolher provedor",
    "activity.grid.pending":
      "As frentes de trabalho aparecem depois que cada hora termina e é resumida. As sessões capturadas estão em Sessões.",
    "activity.grid.loadError": "Não foi possível carregar as frentes de trabalho: {message}",
    "activity.workstream.details": "Detalhes da frente de trabalho",
    "activity.workstream.kicker": "Frente de trabalho",
    "activity.workstream.back": "Voltar ao panorama do dia",
    "activity.workstream.when": "Quando",
    "activity.workstream.break": "Pausa · {duration}",
    "activity.workstream.hourActive": "{duration} ativos nesta hora",
    "activity.workstream.hourUnlocated": "{duration} nesta hora sem horário exato",
    "activity.workstream.whenHint":
      "Horários das sessões em que este trabalho aparece. Uma sessão pode ter mais de uma frente, então não são a duração dela; o tempo ativo acima é.",
    "activity.workstream.done": "O que foi feito",
  },
});
