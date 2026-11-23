# Decisões

- Reusar captura, relógio ativo, eventos de preview e reconciliação existentes.
- A nota continua a unidade de histórico; não criar uma entidade meeting.
- Marcações pertencem à nota e à recording session, persistidas em SQLite,
  com tempo obtido do gravador nativo, inclusive durante pausa.
- Sugestões usam apenas contexto recente da sessão ativa, enviado ao motor
  de Notes configurado em chamada isolada sem ferramentas. Pedido explícito
  reduz custo e mantém o envio sob controle da pessoa. Pausa/stop invalidam
  respostas em trânsito. Falhas são visíveis e não afetam a captura.
- Preview permanece efêmero e separado dos transcripts finais. O pipeline
  atual de geração recebe somente a transcrição definitiva, preservando áudio
  e texto em falhas. Nunca inferir compromissos sem evidência.
- UI estende o editor existente, com tokens, controles, idiomas e hierarquia
  atuais. Fontes são apresentadas como Você / Outros, sem inferir identidade.
- Consulta nativa de disponibilidade informa epoch de privacidade e contexto
  elegível, sem retornar a transcrição. A UI consulta somente enquanto a sessão
  está ativa. Expiração de contexto bloqueia novos pedidos sem apagar respostas
  já solicitadas; pausa/stop/privacidade invalidam o pedido. Marcações concluídas
  notificam os editores da mesma nota, inclusive após navegação durante a escrita.
- Linux comprova contratos, persistência e renderização; captura macOS,
  permissões e inferência paga permanecem provas separadas se indisponíveis.
