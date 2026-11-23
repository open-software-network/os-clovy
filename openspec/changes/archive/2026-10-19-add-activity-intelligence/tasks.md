# Tasks

## 1. S1a — Provedores de LLM (núcleo)

- [x] 1.1 Núcleo `llm` em `src-tauri` com geração pontual (texto/JSON com schema), marcador de autoria, tempo limite sem processo órfão e semáforo de atividade; verificado por testes unitários e de integração com CLIs falsos.
- [x] 1.2 Detecção dos seis CLIs via PATH do shell de login, preservando o ambiente do perfil, com caminho e versão; verificado por teste com executáveis falsos em diretório temporário.
- [x] 1.3 Registro de endpoints OpenAI-compatíveis nomeados com segredo no Keychain e migração do `localGeneration` existente; verificado por testes de carga de settings legado e de DTO sem segredo.
- [x] 1.4 Teste de conexão e probe do nível de saída estruturada por provedor; verificado por integração com CLI falso e servidor HTTP falso.
- [x] 1.5 Seleção por uso (chat, notas, limpeza de ditado, atividade; atividade padrão "nenhum") ligada às rotas existentes de notas, ditado e agente; verificado por testes de roteamento e nota gerada por CLI falso.
- [x] 1.6 UI de provedores na aba Modelos (lista, instalado/não instalado, cadastro de endpoint, testar, escolha por uso) en/pt-BR conforme `spec/`; verificado por testes de componente e captura no app de desenvolvimento.

## 2. S2 — Captura de atividade e banco cifrado

- [x] 2.1 Banco `activity.sqlite3` com SQLCipher, chave no Keychain (`-dev` em debug), migrações próprias, tratamento de chave ausente; verificado por teste de arquivo ilegível sem chave e pela suíte completa do banco principal.
- [x] 2.2 Extração AX com habilitação Chromium/Electron e fallback OCR Vision em memória; verificado por integração e por captura real no app de desenvolvimento.
- [x] 2.3 Agendador de 2 s, monitores secundários (~10 s, opcional) e eventos de entrada sem conteúdo com clipboard redigido; verificado por testes do agendador, normalizador e redator.
- [x] 2.4 Exclusões (apps, domínios, janelas privadas sem OCR, janelas do Clovy, vídeo protegido, horário de trabalho), pausa manual, pausa por pouco disco e registros de pausa; verificado por testes de filtro e relógio simulado.
- [x] 2.5 Retenção (30 dias padrão, só processados) com vácuo incremental; verificado por teste com cursor de processamento.
- [x] 2.6 Aba "Atividade" nas configurações (ligar, permissões com deep links, exclusões, horário, retenção, monitores, vídeo protegido) e estado/pausa no menu da barra, en/pt-BR; verificado por testes de componente e captura no app de desenvolvimento.
- [x] 2.7 Export de depuração do banco de atividade (somente debug, sem expor chave); verificado por teste de integração.

## 3. S3 — Linha do tempo

- [x] 3.1 ETL incremental de quadros para sessões (contexto de domínio/workspace), sessão ativa e cursor; verificado por testes do construtor e de retomada.
- [x] 3.2 Lacunas (>5 min) classificadas e excluídas das durações; verificado por testes de classificação.
- [x] 3.3 Categorizador determinístico de 10 classes com piso de confiança; verificado por testes com amostras por classe.
- [x] 3.4 Estatísticas do dia e busca FTS5 com filtro de período; verificado por testes do agregador e do índice.
- [x] 3.5 Vista "Hoje" na barra lateral (dias, blocos, lacunas, sessão ativa, detalhe, estatísticas, busca, estado vazio) en/pt-BR; verificado por testes de componente e captura no app de desenvolvimento com dados de teste.
- [x] 3.6 Ferramentas do agente `search_activity` e `get_activity_timeline` registradas no catálogo e no despacho; verificado por teste de despacho.

## 4. S5 — Ingestão de sessões de agentes de código

- [x] 4.1 Leitores somente leitura das sete fontes com chave por fonte; verificado por testes com fixtures sintéticas e checagem de hash/mtime.
- [x] 4.2 Normalização e segmentação (1 h de pausa ou de duração, corte em prompt) com ciclo vivo → selado → resumido; verificado por testes do segmentador.
- [x] 4.3 Resumo pelo CLI do agente ou provedor de atividade, filtro por marcador de autoria; verificado por testes do seletor e do filtro.
- [x] 4.4 Faixa de sessões de agentes na vista "Hoje" e seção de fontes na aba Atividade; verificado por teste de componente e captura no app de desenvolvimento.

## 5. S4 — Inteligência do dia

- [x] 5.1 Destilação local com deduplicação semântica por embeddings locais e resgate de entidades; verificado por testes do pipeline.
- [x] 5.2 Relatório por hora com minutos normalizados pelo tempo ativo medido; verificado por teste de normalização e integração com provedor falso.
- [x] 5.3 Frentes de trabalho por dobra incremental ancorada; verificado por teste com resposta simulada.
- [x] 5.4 Resumo do dia (narrativa, insights, standup, painéis ligados aos dados, reuniões/notas do Clovy, blocos de agentes), idioma da interface e chaves JSON em inglês; verificado por integração com provedor falso.
- [x] 5.5 Agendamento diário com recuperação após suspensão e geração sob demanda, inativo sem provedor; verificado por teste com relógio simulado.
- [x] 5.6 Notificações (resumo pronto, falha), horário silencioso, abrir vista, adiar quando suportado; verificado por testes da política e prova real no app de desenvolvimento.
- [x] 5.7 Resumo, frentes e relatórios na vista "Hoje" com copiar standup e estado desligado; verificado por testes de componente e captura no app de desenvolvimento.

## 6. S6 — Servidor MCP do Clovy

- [x] 6.1 Canal local autenticado no app (socket Unix 0600 + segredo) e binário stdio `clovy-mcp` empacotado; verificado por integração com conexão sem segredo recusada.
- [x] 6.2 Ferramentas `search_notes`, `get_note`, `list_dictations`, `list_memories` e de atividade (exceto `get_day_summary`, que depende do S4 e entra em 7.5), recursos `clovy://context` e `clovy://guide`; verificado por chamadas MCP reais contra banco de teste.
- [x] 6.3 Seção na aba Agente (ligar, copiar configuração para Claude Code e Cursor); verificado por teste do gerador e captura no app de desenvolvimento.

## 7. S1b — CLIs e endpoints como motor do chat

- [x] 7.1 Adaptador de sessão por CLI (turno com retomada, tradução do stream para eventos/itens existentes, cancelamento, mensagem seguinte durante execução); verificado por integração com CLIs falsos.
- [x] 7.2 Ferramentas do Clovy via servidor MCP para CLIs que aceitam MCP e indicação na UI quando não aceitam; verificado por integração com CLI falso lendo a configuração MCP.
- [x] 7.3 Endpoint cadastrado como modelo do agente com ferramentas e streaming; verificado por integração com servidor falso com tool call.
- [x] 7.4 Escolha do motor na sessão de chat (UI) en/pt-BR; verificado por teste de componente e conversa real no app de desenvolvimento com um CLI instalado.
- [x] 7.5 Ferramenta MCP `get_day_summary` sobre os resumos do S4; verificado por chamada MCP real contra banco de teste.

## 8. Integração

- [x] 8.1 Integrar recortes no `main` local, `make verify` verde e build de produção (`### Build` do DOX) no HEAD integrado.
- [ ] 8.2 Prova funcional ponta a ponta no app de desenvolvimento: captura ligada, linha do tempo do dia, resumo gerado por provedor escolhido, notificação, cliente MCP real consultando o Clovy, conversa com motor CLI; evidência em `.tmp/verify/`. (Feito em 2026-10-05, exceto o banner da notificação, não observável nesta sessão; ver `run-native/report.md` do ticket de integração.)
- [x] 8.3 Atualizar `AGENTS.md`/docs (`docs/index.md`, nova doc de atividade) e `CONTEXT.md` com os termos novos.
