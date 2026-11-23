import { defineMessages } from "../define";

export default defineMessages({
  en: {
    "codingAgents.settings.title": "Coding agents",
    "codingAgents.settings.description":
      "Read the sessions you run with coding agents on this device and turn them into summarized blocks on your timeline. Their files are only read, never changed, and the blocks are kept in the encrypted activity database.",
    "codingAgents.settings.found": "Found on this device",
    "codingAgents.settings.notFound": "Not found on this device",
    "codingAgents.settings.toggleAria": "Read {name} sessions",
    "codingAgents.settings.summaries":
      "Each block is summarized by the agent's own CLI when it is installed and can run with its tools turned off (Claude Code), otherwise by the activity provider chosen in Settings, Models. With neither, blocks stay without a summary and nothing leaves this device.",
    "codingAgents.settings.lastScan": "Last read at {time}",
    "codingAgents.settings.databaseUnavailable":
      "The activity database is not available, so sessions cannot be stored. Check the status above.",
    "codingAgents.settings.error": "Last read failed: {message}",

    "codingAgents.lane.title": "Coding agents",
    "codingAgents.lane.listAria": "Coding agent sessions",
    "codingAgents.lane.empty": "No coding agent sessions on this day",
    "codingAgents.lane.timeRange": "{start} to {end}",
    "codingAgents.lane.noProject": "No project",
    "codingAgents.lane.live": "In progress",
    "codingAgents.lane.waiting": "Waiting for a summary",
    "codingAgents.lane.prompts": {
      one: "{count} prompt",
      other: "{count} prompts",
    },
  },
  "pt-BR": {
    "codingAgents.settings.title": "Agentes de código",
    "codingAgents.settings.description":
      "Leia as sessões que você faz com agentes de código neste dispositivo e transforme-as em blocos resumidos na sua linha do tempo. Os arquivos deles são apenas lidos, nunca alterados, e os blocos ficam no banco de atividade cifrado.",
    "codingAgents.settings.found": "Encontrado neste dispositivo",
    "codingAgents.settings.notFound": "Não encontrado neste dispositivo",
    "codingAgents.settings.toggleAria": "Ler sessões do {name}",
    "codingAgents.settings.summaries":
      "Cada bloco é resumido pelo CLI do próprio agente quando ele está instalado e pode rodar com as ferramentas desligadas (Claude Code) e, caso contrário, pelo provedor de atividade escolhido em Configurações, Modelos. Sem nenhum dos dois, os blocos ficam sem resumo e nada sai deste dispositivo.",
    "codingAgents.settings.lastScan": "Última leitura às {time}",
    "codingAgents.settings.databaseUnavailable":
      "O banco de atividade não está disponível, então as sessões não podem ser guardadas. Confira o status acima.",
    "codingAgents.settings.error": "A última leitura falhou: {message}",

    "codingAgents.lane.title": "Agentes de código",
    "codingAgents.lane.listAria": "Sessões de agentes de código",
    "codingAgents.lane.empty": "Nenhuma sessão de agente de código neste dia",
    "codingAgents.lane.timeRange": "{start} a {end}",
    "codingAgents.lane.noProject": "Sem projeto",
    "codingAgents.lane.live": "Em andamento",
    "codingAgents.lane.waiting": "Aguardando resumo",
    "codingAgents.lane.prompts": {
      one: "{count} prompt",
      other: "{count} prompts",
    },
  },
});
