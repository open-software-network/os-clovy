# Tasks

Main atualiza checkboxes somente após confrontar a prova. Gates/build/Cargo são serializados; workers não executam checks durante edição concorrente da onda. Prova em sandbox/HTTP não conta como serviço externo/native real. Tasks externas ficam abertas sem chaves/ação ou autorização de custo do dono.

## 1. Contratos compartilhados e backend áudio/texto

- [ ] 1.1 Migrar domínio/config/headers/credenciais/funding para Groq, OpenRouter, fal e Tavily, retirando branches Venice e exemplos antigos; verificar isolamento de credencial e cobrança própria/serviço com regressões comportamentais e docs/config atualizados, sem ler valores reais.
- [ ] 1.2 Substituir ASR por Groq/Whisper real nos handlers de ditado/notas, preservando idioma/contexto/bytes/preview e erros; verificar multipart/parser/modelo/no_speech/limites e a regressão de modelo disponível com configuração/BYOK, documentando receita e defaults.
- [ ] 1.3 Implementar texto OpenRouter e catálogo/preços/capacidades reais para chat, ferramentas, geração de notas e cleanup; verificar JSON/SSE/uso/erros e seleção compatível, documentando política de privacidade sem claims Venice.
- [ ] 1.4 Integrar services/HTTP/bootstrap/metering e novos adapters de mídia/web após os contratos compartilhados; verificar contrato dos endpoints existentes e ausência de rotas Venice, mantendo todas as capacidades do pedido.

## 2. Mídia fal e web Tavily

- [ ] 2.1 Implementar geração e edição de imagem por endpoints fal oficiais distintos, com preferências de segurança suportadas e download final limitado; verificar input/result/erros em sandbox e registrar schema/modelos/preços na documentação da capacidade.
- [ ] 2.2 Implementar geração de vídeo e animação de imagem por fal até o asset final, preservando parâmetros suportados e identidade/transições do job; verificar queued/running/completed/error, asset/URL/tamanho/settlement e documentar a receita real de ambos os fluxos.
- [ ] 2.3 Implementar Tavily search e extract nas ferramentas existentes, removendo o aggregator Venice; verificar resultados/failed-results/erro/credencial/URL e limits em sandbox, com docs de configuração e prova externa qualificadas.

## 3. Desktop, migração e superfícies

- [ ] 3.1 Migrar comandos/DTOs/registro IPC e todos os callsites de settings/modelos/funding/privacy sem aliases Venice; verificar references de exports, testes consumer-visible e os quatro modos do picker, com docs dos novos nomes e interfaces.
- [ ] 3.2 Migrar settings globais e overrides legados para defaults/modelos válidos, preservando CLI/endpoints, segredos de endpoint e história; verificar leitura/save/restart em store/cópia sandbox e documentar o tratamento de seleção/credencial legadas sem transferir segredo Venice.
- [ ] 3.3 Integrar configuração/estado de credenciais dos quatro provedores no store canônico e forwarding somente para a ação correspondente; verificar falta/rejeição de chave, metering/BYOK e DTO sem segredo com stores isolados, sem tocar Keychain real por Main/worker.
- [ ] 3.4 Atualizar catálogos/logos/i18n/sugestões/companion e os fluxos de ditado/notas/chat/imagem/vídeo/web; verificar UI efetiva com dados sandbox e remover caminhos ativos Venice sem reescrever ADRs e histórico aceitos.

## 4. Captura estável e integração no dev

- [ ] 4.1 Corrigir o comparador OCR que admite ciclos com tolerância pairwise, mantendo ordem de leitura top-to-bottom/left-to-right sem suppress/catch; obter RED de ordem de leitura e GREEN de permutations/linhas ancoradas, atualizar doc comment e recipe de comprovação da captura contínua.
- [ ] 4.2 Main confrontar fontes/reports/review, executar busca exaustiva qualificada de dependências Venice e refresh do Graft; verificar cada cenário sem usar ausência de string como teste permanente ou apagar história.
- [ ] 4.3 Integrar o escopo completo em main, capturar gate/build/verify-cli-worker final no mesmo fingerprint e validar as três changes OpenSpec e Test stamps; preservar falhas históricas e não declarar PASS-UNPROVEN como aceite.
- [ ] 4.4 Restaurar preparo dev/certificado/ids estáveis após produção/gate e abrir dev normal para o dono; observar janela, helper, grants, continuidade da captura/MCP, restart/rebuild/HMR e skills em perfil vazio sem fixture no perfil normal.

## 5. Prova externa completa e aceite

- [ ] 5.1 Dono configurar credenciais localmente e executar ou autorizar a prova de áudio Groq: Fn com voz/transcrição/inserção e nota/preview em sandbox; observar os resultados reais sem segredo ou áudio privado na evidência.
- [ ] 5.2 Executar com ação/autorização de custo do dono a prova real OpenRouter de chat/ferramentas, geração de nota e cleanup remoto; verificar outputs/estado/persistência e que CLI/endpoints preservados continuam funcionando.
- [ ] 5.3 Executar com ação/autorização de custo do dono as quatro capacidades fal: gerar/editar imagem, gerar vídeo e animar imagem; observar assets finais utilizáveis, erros/transições e persistência em sandbox.
- [ ] 5.4 Executar com ação/autorização de custo do dono search/extract Tavily e observar os retornos reais pelas ferramentas, sem mascarar ausências de chave/serviço por mocks.
- [ ] 5.5 Concluir review de risco/privacidade/credenciais proporcional, eixos code/review/reality e verify --record por sessão independente; Main aceitar somente o pedido completo e critérios anteriores de captura/MCP/opt-outs, com destino dos recursos e publicação apenas autorizada.
