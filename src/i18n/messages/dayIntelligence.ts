import { defineMessages } from "../define";

export default defineMessages({
  en: {
    "dayIntelligence.tab.timeline": "Timeline",
    "dayIntelligence.tab.summary": "Day summary",
    "dayIntelligence.tabs.label": "Today sections",

    "dayIntelligence.summary.title": "Day summary",
    "dayIntelligence.generate": "Generate summary",
    "dayIntelligence.regenerate": "Generate again",
    "dayIntelligence.generating": "Generating summary",
    "dayIntelligence.copyStandup": "Copy standup",
    "dayIntelligence.copied": "Standup copied",
    "dayIntelligence.generatedAt": "Generated at {time} by {provider}",
    "dayIntelligence.empty":
      "No summary for this day yet. It is generated at {time}, or you can generate it now.",
    "dayIntelligence.error": "Could not generate the summary: {message}",
    "dayIntelligence.loadError": "Could not load the day summary: {message}",
    "dayIntelligence.errors.incomplete":
      "Part of this day is still being processed, so the summary was not written. Try again in a few minutes.",
    "dayIntelligence.errors.noActivity": "There is no activity to summarize for this day yet.",
    "dayIntelligence.embedderDownloading":
      "Downloading the local text model (once, about 130 MB). Summaries work meanwhile.",

    "dayIntelligence.provider.missingTitle": "Day summaries need an activity provider",
    "dayIntelligence.provider.missingDescription":
      "Choose an activity provider in Settings, Models. Until then nothing is generated and no activity text leaves this computer.",
    "dayIntelligence.provider.insufficientTitle": "The activity provider needs JSON schema support",
    "dayIntelligence.provider.insufficientDescription":
      "Test the provider in Settings, Models, or choose one that supports JSON schema.",
    "dayIntelligence.provider.choose": "Choose a provider",

    "dayIntelligence.standup.title": "Standup",
    "dayIntelligence.standup.done": "Done",
    "dayIntelligence.standup.inProgress": "In progress",
    "dayIntelligence.standup.blockers": "Blockers",
    "dayIntelligence.standup.none": "None",
    "dayIntelligence.insights.title": "Insights",

    "dayIntelligence.panels.title": "The day in numbers",
    "dayIntelligence.panels.meetings": "Meetings",
    "dayIntelligence.panels.meetingCount": {
      one: "{count} meeting",
      other: "{count} meetings",
    },
    "dayIntelligence.panels.codingAgents": "Coding agents",
    "dayIntelligence.panels.codingAgentBlocks": {
      one: "{count} session",
      other: "{count} sessions",
    },
    "dayIntelligence.panels.byHour": "Focus by hour",
    "dayIntelligence.panels.byWorkstream": "Time by workstream",

    "dayIntelligence.workstreams.title": "Workstreams",
    "dayIntelligence.workstreams.empty": "Workstreams appear as each hour is reported.",
    "dayIntelligence.hours.title": "Hour by hour",
    "dayIntelligence.hours.empty": "Each completed hour with activity gets a report.",

    "dayIntelligence.settings.title": "Day summary",
    "dayIntelligence.settings.description":
      "Clovy turns your activity into hour reports, workstreams, and a summary of the day with your activity provider.",
    "dayIntelligence.settings.time": "Summary time",
    "dayIntelligence.settings.timeDescription":
      "Generated once a day at this time, or as soon as this device wakes up afterward.",
    "dayIntelligence.settings.notifications": "Notifications",
    "dayIntelligence.settings.notificationsDescription":
      "Notify when the day summary is ready or activity stops working.",
    "dayIntelligence.settings.quietHours": "Quiet hours",
    "dayIntelligence.settings.quietHoursDescription":
      "Notifications raised in quiet hours wait until they end.",
    "dayIntelligence.settings.quietStart": "Quiet from",
    "dayIntelligence.settings.quietEnd": "Quiet until",
  },
  "pt-BR": {
    "dayIntelligence.tab.timeline": "Linha do tempo",
    "dayIntelligence.tab.summary": "Resumo do dia",
    "dayIntelligence.tabs.label": "Seções de hoje",

    "dayIntelligence.summary.title": "Resumo do dia",
    "dayIntelligence.generate": "Gerar resumo",
    "dayIntelligence.regenerate": "Gerar de novo",
    "dayIntelligence.generating": "Gerando resumo",
    "dayIntelligence.copyStandup": "Copiar standup",
    "dayIntelligence.copied": "Standup copiado",
    "dayIntelligence.generatedAt": "Gerado às {time} por {provider}",
    "dayIntelligence.empty":
      "Ainda não há resumo deste dia. Ele é gerado às {time}, ou você pode gerar agora.",
    "dayIntelligence.error": "Não foi possível gerar o resumo: {message}",
    "dayIntelligence.loadError": "Não foi possível carregar o resumo do dia: {message}",
    "dayIntelligence.errors.incomplete":
      "Parte deste dia ainda está sendo processada, então o resumo não foi escrito. Tente de novo em alguns minutos.",
    "dayIntelligence.errors.noActivity": "Ainda não há atividade para resumir neste dia.",
    "dayIntelligence.embedderDownloading":
      "Baixando o modelo de texto local (uma vez, cerca de 130 MB). Os resumos funcionam enquanto isso.",

    "dayIntelligence.provider.missingTitle": "O resumo do dia precisa de um provedor de atividade",
    "dayIntelligence.provider.missingDescription":
      "Escolha um provedor de atividade em Configurações, Modelos. Até lá nada é gerado e nenhum texto de atividade sai deste computador.",
    "dayIntelligence.provider.insufficientTitle":
      "O provedor de atividade precisa de suporte a JSON schema",
    "dayIntelligence.provider.insufficientDescription":
      "Teste o provedor em Configurações, Modelos, ou escolha um que suporte JSON schema.",
    "dayIntelligence.provider.choose": "Escolher provedor",

    "dayIntelligence.standup.title": "Standup",
    "dayIntelligence.standup.done": "Feito",
    "dayIntelligence.standup.inProgress": "Em andamento",
    "dayIntelligence.standup.blockers": "Bloqueios",
    "dayIntelligence.standup.none": "Nenhum",
    "dayIntelligence.insights.title": "Observações",

    "dayIntelligence.panels.title": "O dia em números",
    "dayIntelligence.panels.meetings": "Reuniões",
    "dayIntelligence.panels.meetingCount": {
      one: "{count} reunião",
      other: "{count} reuniões",
    },
    "dayIntelligence.panels.codingAgents": "Agentes de código",
    "dayIntelligence.panels.codingAgentBlocks": {
      one: "{count} sessão",
      other: "{count} sessões",
    },
    "dayIntelligence.panels.byHour": "Foco por hora",
    "dayIntelligence.panels.byWorkstream": "Tempo por frente",

    "dayIntelligence.workstreams.title": "Frentes de trabalho",
    "dayIntelligence.workstreams.empty": "As frentes aparecem conforme cada hora é relatada.",
    "dayIntelligence.hours.title": "Hora a hora",
    "dayIntelligence.hours.empty": "Cada hora concluída com atividade ganha um relatório.",

    "dayIntelligence.settings.title": "Resumo do dia",
    "dayIntelligence.settings.description":
      "O Clovy transforma sua atividade em relatórios por hora, frentes de trabalho e um resumo do dia com o seu provedor de atividade.",
    "dayIntelligence.settings.time": "Horário do resumo",
    "dayIntelligence.settings.timeDescription":
      "Gerado uma vez por dia neste horário, ou assim que o dispositivo voltar da suspensão após esse horário.",
    "dayIntelligence.settings.notifications": "Notificações",
    "dayIntelligence.settings.notificationsDescription":
      "Avisar quando o resumo do dia estiver pronto ou a atividade parar de funcionar.",
    "dayIntelligence.settings.quietHours": "Horário silencioso",
    "dayIntelligence.settings.quietHoursDescription":
      "Notificações geradas no horário silencioso esperam até ele terminar.",
    "dayIntelligence.settings.quietStart": "Silencioso a partir de",
    "dayIntelligence.settings.quietEnd": "Silencioso até",
  },
});
