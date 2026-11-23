import { defineMessages } from "../define";

export default defineMessages({
  en: {
    "mcpServer.tab": "MCP",
    "mcpServer.heading": "Clovy MCP server",
    "mcpServer.description":
      "Connect local clients to your notes, dictations, memories, and activity. Protected actions require approval in Clovy.",
    "mcpServer.approvals.heading": "External approvals",
    "mcpServer.approvals.indicator": {
      one: "{count} MCP approval",
      other: "{count} MCP approvals",
    },
    "mcpServer.approvals.profile": "Data set",
    "mcpServer.approvals.approve": "Approve once",
    "mcpServer.approvals.deny": "Deny",
    "mcpServer.approvals.loading": "Loading pending requests…",
    "mcpServer.approvals.empty": "No pending requests.",
    "mcpServer.approvals.loadError": "Could not load external approval requests.",
    "mcpServer.approvals.approved": "Request approved.",
    "mcpServer.approvals.denied": "Request denied.",
    "mcpServer.approvals.unavailable": "The request could not be decided. It may have expired.",
    "mcpServer.toggleTitle": "Turn on MCP server",
    "mcpServer.toggleDescription":
      "Answers only while Clovy is open. Activity tools appear when activity capture is on.",
    "mcpServer.toggleAria": "Clovy MCP server",
    "mcpServer.running": "Running",
    "mcpServer.binaryMissing":
      "The clovy-mcp program was not found next to Clovy. Install a full build to use the server.",
    "mcpServer.startError": "The server could not start: {message}",
    "mcpServer.updateError": "Could not update the MCP server.",
    "mcpServer.claudeCode.title": "Claude Code",
    "mcpServer.claudeCode.description":
      "Run this command, or add the JSON to a project's .mcp.json.",
    "mcpServer.cursor.title": "Cursor",
    "mcpServer.cursor.description": "Add this to ~/.cursor/mcp.json.",
    "mcpServer.copy": "Copy",
    "mcpServer.copied": "Copied",
    "mcpServer.copyCommand": "Copy Claude Code command",
    "mcpServer.copyClaudeCodeConfig": "Copy Claude Code configuration",
    "mcpServer.copyCursorConfig": "Copy Cursor configuration",
  },
  "pt-BR": {
    "mcpServer.tab": "MCP",
    "mcpServer.heading": "Servidor MCP do Clovy",
    "mcpServer.description":
      "Conecte clientes locais às suas notas, ditados, memórias e atividade. Ações protegidas exigem aprovação no Clovy.",
    "mcpServer.approvals.heading": "Aprovações externas",
    "mcpServer.approvals.indicator": {
      one: "{count} aprovação MCP",
      other: "{count} aprovações MCP",
    },
    "mcpServer.approvals.profile": "Conjunto de dados",
    "mcpServer.approvals.approve": "Aprovar uma vez",
    "mcpServer.approvals.deny": "Negar",
    "mcpServer.approvals.loading": "Carregando pedidos pendentes…",
    "mcpServer.approvals.empty": "Nenhum pedido pendente.",
    "mcpServer.approvals.loadError": "Não foi possível carregar os pedidos de aprovação externa.",
    "mcpServer.approvals.approved": "Pedido aprovado.",
    "mcpServer.approvals.denied": "Pedido negado.",
    "mcpServer.approvals.unavailable":
      "Não foi possível registrar sua decisão. O pedido pode ter expirado.",
    "mcpServer.toggleTitle": "Ligar servidor MCP",
    "mcpServer.toggleDescription":
      "Responde só enquanto o Clovy está aberto. As ferramentas de atividade aparecem quando a captura de atividade está ligada.",
    "mcpServer.toggleAria": "Servidor MCP do Clovy",
    "mcpServer.running": "Em execução",
    "mcpServer.binaryMissing":
      "O programa clovy-mcp não foi encontrado junto ao Clovy. Instale uma versão completa para usar o servidor.",
    "mcpServer.startError": "O servidor não pôde iniciar: {message}",
    "mcpServer.updateError": "Não foi possível atualizar o servidor MCP.",
    "mcpServer.claudeCode.title": "Claude Code",
    "mcpServer.claudeCode.description":
      "Execute este comando ou adicione o JSON ao .mcp.json de um projeto.",
    "mcpServer.cursor.title": "Cursor",
    "mcpServer.cursor.description": "Adicione isto a ~/.cursor/mcp.json.",
    "mcpServer.copy": "Copiar",
    "mcpServer.copied": "Copiado",
    "mcpServer.copyCommand": "Copiar comando do Claude Code",
    "mcpServer.copyClaudeCodeConfig": "Copiar configuração do Claude Code",
    "mcpServer.copyCursorConfig": "Copiar configuração do Cursor",
  },
});
