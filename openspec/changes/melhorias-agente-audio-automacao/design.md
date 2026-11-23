# Design

## Context

Ver proposal.md e os backlogs QA. Base de produto `218ab47d842b91da3da1be1fc3883b9b74bace69`; docs de escopo em `30a0cb13ce6878cc3e9e5860906e152866e7bd12`. Investigação read-only separou contratos e verificou fontes atuais. Linux não compila o crate desktop por credentials/keyring cfg macOS/Windows; erro já observado no baseline. Box é ambiente de construção, não nova plataforma de produto. Frontend/checks/módulos portáveis/motores Linux podem ser exercitados lá; testes do crate e usos nativos ficam registrados para Mac/Windows.

## Goals / Non-Goals

**Goals:** completar T2–T7 sem criar segunda convenção, preservar dados/autoridade e permitir integração independente com revisão central.

**Non-Goals:** portar Parakeet ou o app ao Linux, corrigir categorias manualmente, mover memória entre escopos, substituir contas/credenciais herdadas de CLI, instalar motores globalmente ou oferecer MCP upstream irrestrito. Não ampliar seleção de endpoint Windows sem reprodução F4.

## Decisions

### D1 — Ownership e ordem

Worktrees/branches separadas a partir do snapshot de escopo; commits dos workers não vão direto à main. Donos:
- T1 já separado nas duas changes existentes: migration54, Dock/app_preferences e dictation app metadata/contexto.
- T2 áudio: `audio/system_windows.rs`, reconciliação TS/eventos de descarte, NoteEditor/opções e testes/docs áudio. Não mexe em provider routing sem causa comprovada.
- T3 autorização/contexto: approval_broker, persistência de política dos consumidores internos em agent_sessions/chat_engine e contexto MCP. A migração55 fica como histórico. A interface de aprovação externa pertence a `retirar-chat-do-clovy`. Não controla CRUD MCP nem adaptadores de motor.
- T4 gestão nativa: MCP tool groups, port/service de domínio e sincronização UI, skills discovery/admin, contrato de nomes/catálogo/guia A13. Não implementa bypass; chama broker central. Ajustes pequenos nos arquivos T3/T1 restritos aos seus hunks e reconciliados pelo integrador.
- T5D desktop: helper/driver/computer_use, pin/scripts/packaging/tests/docs desse motor. T5B browser: browser/broker/proxy/launcher/transporte, schemas browser, grant/UI e remoção exclusiva extensão. Ambos seguem D6.
- T6 OAuth: agent_mcp/cofre lifecycle, gateway/settings e diagnóstico de origem externo; não muda credentials alheias.
- T7 integrador: reconcilia lib.rs/commands/tauri/types/events/packaging/docs/index/ADRs, registra grupos, aplica interface T3 e perfil T4 nos motores, verifica contrato de todos, revisão/segredos/main e provas.

Migration54 é T1; 55 é T3. Nenhuma outra migration sem reserva central. Alterações de libs/types/ADRs compartilhadas são hunks da frente, não refatoração. Snapshots isolados não provam conjunto: T7 faz verificação integrada após merge. Compilações/suítes na box serializadas por flock; target compartilhado sem incremental/debug info, sem limpar trabalho alheio.

### D2. Política efetiva e recibos por execução

Resolver a autoridade a partir da política de execução capturada pelo host nos consumidores internos. O campo legado `approval_mode` permanece sem autoridade. Mudanças de configuração se aplicam ao próximo run, e a interface de Rotinas mantém suas opções existentes. O host recusa mutações que exigem aprovação em runs não assistidos. Clientes externos seguem o contrato de autorização por chamada de `retirar-chat-do-clovy`.

`ApprovalBroker::effective_policy(&ApprovalTurn) -> Result<EffectiveApprovalPolicy, ApprovalDenial>` resolve vínculo/run ativo/snapshot host-owned. Resultado `{ mode, session_id, run_id }`; snapshot legado ausente ou unattended nunca vira bypass. `request` consome a mesma resolução. `ApprovalOutcome::Approved` permanece; recibo ganha `authorizationOrigin: user_once|session_bypass`, `authorizationPolicyRunId`, `resolvedBy: desktop|policy`, mantendo correlação engine/session/run/tool/call e resolution once exigidas pelos conectores. Persistência falha => nenhum efeito. Clarificação nunca autoaprova. session/always inválidos. Não alterar argv/sandbox/network dos CLIs para implementar esta camada.

### D3 — Escopo bound e gestão tipada

`ToolCtx` deve conter a partição congelada pelo host no `dataPartition` de `run_config_json`, capturada na transação do run e transportada em `TurnContext`/`TurnBinding`, sem depender da seleção atual da UI de `app_data()`. Reassociar a conversa invalida o acesso do vínculo antigo; somente um novo run captura a nova associação. Snapshot ausente/malformado ou associação divergente é recusa, inclusive após aprovação/lock e antes de publicar contexto ou despachar prompt. Sessão sem associação captura `default` explicitamente; rotina conserva a origem da própria sessão. Cliente externo preserva sua superfície de leitura e recebe somente as ferramentas com autoridade própria prevista em `retirar-chat-do-clovy`. Revalidar vínculo/objeto/origem/destino e run após toda espera; projeto é organização, não grant transversal por ID. Não autorizar cross-profile por campo do modelo. Atividade é da instalação. Recusar autoexclusão da conversa bound e autocancelamento do próprio run.

Contrato de nomes desta expansão (manter existentes não substituídos; atualizar documento dono/catálogo junto):

| Área | Tools |
|---|---|
| Notas | list_notes, create_note, update_note, delete_note; get_note/search_notes existentes; list_note_transcripts |
| Conversas | list_conversations, get_conversation, list_conversation_messages, search_conversation_messages, list_conversation_artifacts, read_conversation_artifact, get_conversation_run, create_conversation, rename_conversation, archive_conversation, restore_conversation, delete_conversation, cancel_conversation_run |
| Projetos | list_projects, get_project, create_project, update_project, delete_project, assign_note_project, remove_note_project, assign_conversation_project, remove_conversation_project |
| Rotinas | run_routine, list_routine_runs; manter CRUD/pause/resume/list existentes e ampliar timezone/model/enabledToolsets sem grants implícitos |
| Memória | update_memory, search_memories; manter save/forget/list |
| Atividade | get_activity_settings, update_activity_settings, pause_activity, resume_activity, generate_day_summary |
| Ditado | delete_dictation, start_dictation, stop_dictation, cancel_dictation, get_dictation_status; manter list/get |
| Recorder | pause_recording, resume_recording, recover_recording, retry_note_processing, list_recording_artifacts; manter start/stop/recording_status (nome existente é exceção expressa de spec/mcp-tool-naming.md, não renomear sem necessidade) |
| Skills | list_skill_settings, read_skill, update_skill, enable_skill, disable_skill; manter list_skills/load_skill |

Schemas fechados, limites/cursors tipados e respostas completas paginadas; sem passthrough JSON/helper/path/SQL. Cursor de conversa inclui ordem de run/sequence/id, nunca apenas sequence que reinicia a cada run. Artefato por conversationId+artifactId e offsets/limites de texto, cap de página, erro binário explícito. Busca só mensagens humanas/assistente, não secrets/tool payloads. Não anunciar export/download inexistente.

Notas: introduzir operação transacional `update_note_if_revision(profile,id,expected_revision,patch)` com comparação no UPDATE e erro `note_conflict`; UI/autosave não perde suas semânticas existentes. Novo MCP exige expectedRevision e retorna atual/releitura possível. Projetos update composto atômico; exclusão deleteNotes explícita e preview. Rotina run retorna started/busy/blocked reais, histórico paginado, claim único e modelo CLI resolvido. Memória update preserva ID/criação/escopo e lock de habilitação. Config de atividade preserva campos internos não editáveis e erros de geração atuais.

Dictation lifecycle precisa porta tipada sobre correlação takeId/composer/destino já autorizado, start/status respondem confirmação real, stop finaliza/entrega, cancel descarta tomada correta. Recorder amplia bridge da UI, preserva IDs/duração/fontes; recovery enum validate/discard; list artifacts não concede path. Emitir eventos Tauri existentes/estreitos para a UI invalidar e reler notas, projetos, memórias e skills nas áreas correspondentes. As provas de gestão de sessões e artefatos usam a API autorizada e armazenamento sintético.

### D4 — Transcrição e previews

Compartilhar seleção de transcripts autoritativos entre leitura/busca/geração, mantendo registros legados. Para mesma sessão/fonte com turnos definidos, não repetir agregado substituído; registros sem sessão continuam por identidade, sem dedup textual que apagaria repetições legítimas. Ordenar por sessão/tempo/turn index/created/id e paginar sem duplicação. Snippet vem do transcript realmente correspondente.

F2 usa decisão terminal explícita de spans descartados do pipeline; nunca inferir descarte de final ausente nem remover preview de outra fonte/intervalo/sessão. Evento/DTO comunica descarte no mesmo contrato de sessão/job. F3 separa relógio ativo de pausa: stop pausado não pad até now, resume reinicia marcador e silêncio ativo permanece. F1 habilita opções por capacidade e Windows, sem mostrar botões de TCC macOS nesse sistema. F4/I1 seguem investigação sintética, não causa presumida.

### D5 — Skills e diagnóstico

Uma descoberta/identidade compartilhada em agent_sessions skills/skill_identity para UI/MCP/admin. Ignorar entradas iniciadas por ponto e backups editoriais objetivos (.bak/.old/~), preservar aliases/precedência/disabled e raízes canônicas; não ocultar nomes legítimos foo-backup por regex inventada. Backups observados eram `.creative-*.backup-<timestamp>` (todos ocultos). Global continua read-only; managed content <=512KiB e IDs ASCII alnum/-/_ até128. Symlink só lê alvo canônico permitido na raiz aceita; escrita não escapa raiz managed.

O contexto vinculado deriva do vínculo persistido e inclui identidade, política e superfície reais. O cliente externo recebe apenas seu próprio contexto autorizado. O guia distingue leitura, mutações, custos e políticas de cada superfície. Erros tipados têm origem, fase, código e disposição de entrega; não reduzir tudo a computer_use_failed/agent_mcp_failed. Status não faz login/grant/ação. Temporal: intervalos completos sobrepostos vs totais recortados e ETL materializada são comportamento deliberado; documentar sem alterar cálculo.

### D6 — Motores privados e modalidade

Implementação MCP Windows (2026-11-02): canal local named pipe duplex byte/overlapped com DACL protegida usuário/SYSTEM e rejeição remota, namespace por diretório canônico da instalação e TokenUser real dos peers antes de secret/token. Núcleo de framing/cotas/revogação/recibos e relay compartilhados com Unix; o relay é filho da CLI e não recebe regra parentPID do Computer use. Sem replay após escrita parcial/incerta. Runtime de IO permanece vivo, writes nativos são cancelados explicitamente no aborto e fechamento normal espera conclusão sob prazo. Diretório/secret privados desde criação, objetos ambíguos/reparse recusados sem reparar ACL ancestral. Resolver e pacote incluem clovy-mcp.exe real do Cargo/target/profile, assinatura configurada e proveniência/hash exigidos no NSIS. Mantém ActivityState/defaults/opt-out, McpLaunch estruturado e grants existentes. Prova GNU/Unix não qualifica ACL/IO/NSIS nem turnos de conta no Windows; runtime nativo continua pendente.

Dono autorizou em 2026-11-01: background por padrão, modo visível quando impossível no background em desktop/browser. Preservar ações semânticas headless; seleção de modalidade antes do efeito ou após `not_attempted/unsupported` definitivo. Jamais replay se delivered/unknown. Exibir modo efetivo, não tratar isso como admin/UAC/TCC/auth. Capabilities e campos sensíveis permanecem. Nada de fallback ao motor antigo.

Desktop: engine source pin `vercel-labs/agent-desktop@9eb9b296192fcb70bda25e918cd6dd66abfa3d0d` (declara1.0.2 mas não confundir com tag a47ed...). Encapsular CLI no helper autenticado existente, não FFI no Tauri. macOS iniciar command host stdio privado diretamente, não CLI normal que daemoniza em grupo próprio/socket same-UID. Preservar peer assinado/capability e Stage Manager janela exata em glue nativo estreito Clovy. Windows helper/canal privado autentica pai/imagem/assinatura e Job Object kill-on-close, sem breakaway; identidade HWND/PID/creation/path. Resolver display-name nativamente quando upstream não resolve. Refs/targets por run/epoch; coordenadas imagem->global/DPI verificadas antes de modo físico. Screenshot+AX/vision/SoM e apresentação existentes devem ter paridade explícita, não ser omitidos. Artefatos compilados do pin, SHA/licença/origem/stamp, assinatura e TCC real qualificados no Mac/Windows.

Browser: engine source pin `vercel-labs/agent-browser@5868f7c05020e047c00e68e4abf9253cce864f41`. Build downstream estreito/patch auditável para host privado autenticado, sem streaming/dashboard/CDP público, plugins/WebMCP/autoconnect. Preservar engine upstream; não reimplementar browser com adapter fake. Acrescentar inspect_reference/guarded_action privados retornando origin/document/backendNode/InteractiveElement e comparar expected exato sem recuperação role/name. Namespace gerado pelo host não é auth. BrowserTransport recebe dono bound, reserva cancelável antes de startup, supervisor conhece PID/grupo do Chrome antes do wait CDP, shutdown não depende de mutex da chamada.

Perfil assistido persistente exclusivo app_paths com lease único; rotina perfil efêmero, limite2 e idle5min como atual. Identidade da aba mapeia ID Clovy para tN+targetId, sem fallback se tab_gone; refs invalidadas no fim do run. Login manual/takeover suspende agente/capturas; perfil persiste após Stop. Storage nativo apropriado, sem --password-store=basic/--use-mock-keychain para perfil persistente, sem expor vault/cookies ou salvar senhas pelo produto. Proxy/DNS pinning/redes privadas/flags de rotinas preservados, não substituídos por --allowed-domains; CONNECT é túnel, não inspeção HTTPS.

Revisão de segurança 2026-11-01: metadados/rótulos de página não provam ausência de efeito. Todo click/fill/press pede ApprovalBroker central, com expected exato retido e revalidado depois; campos sensíveis continuam HumanOnly inclusive click em senha mostrada como texto. Preview mostra label sanitizada/capada/quoted como texto não confiável da página e origem, sem valores. Rotinas conservam leitura/navegação explícita, mas não input sem pessoa; não há bypass próprio do browser. Erro de rede posterior a mutação entregue termina tarefa/handles e preserva recibo unknown/do-not-repeat, nunca refused pré-efeito. Captura precisa cobrir conteúdo editável herdado/overflow e snapshot não pode depender de inventário de valores defasado em relação ao AX.

Ligar em startup configure_transport + access flag + outcome repository + raízes de artefatos, e teardown em run/shutdown. Corrigir schemas/callers: tab_id integer obrigatório onde aplicável, fill.text, press.ref, back implementado; remover accept_shared_tab. Extensão atual é exclusiva desse fluxo, mas mover `extension_host::production_app_data_dir` para app_paths antes de remover host, pois stdio MCP protege dados de produção. Retirar dependências/empacotamento antigo só no cutover completo. Chrome for Testing pinado com hashes reais no manifest de preparação (nunca install latest em runtime); versões/commits recentes exigem justificativa de revisão visível conforme política de dependências, não exclusão genérica/cooldown desligado.

### D7 — Vercel e OAuth gerenciado

Reusar AgentMcpSubsystem/StreamableHTTP/cofre/gateway. Preset https://mcp.vercel.com inicialmente não conectado; não nova stack de auth. Discovery valida issuer/HTTPS, PKCE S256/state/loopback, callback recebe code. Registrar identidade Clovy, sem reaproveitar identidade de cliente aprovado. Acrescentar cancel/reconnect/disconnect/revoke, saúde e geração/CAS para refresh não ressuscitar conexão removida. Erro de revogação remota e limpeza local reportados separadamente, nunca falso disconnect limpo.

Remover replay indiscriminado de mutações após401/refresh no genérico; só discovery/leitura seguramente idempotente pode repetir por contrato. Manter recibo/revisão das conexões e scopes live. OAuth public metadata resource anuncia openid, AS também offline_access: refresh depende de grant real e aceite Vercel. Não executar login sem usuário nem alegar aceite do app baseado em Claude/Codex estarem aprovados. CLI externo codex_apps e Atlassian são outra custódia: atribuir origem/erro/reconexão ao proprietário; sem mudar keys nativas nem inventar implementação de namespace. Prova real Vercel bloqueada até provider aceitar cliente e consentimento local.

## Risks / Trade-offs

- Grande superfície shared (lib.rs/commands/tauri/agent_sessions/packaging): fronteiras acima e revisão integrada obrigatórias; sem merge direto de worker.
- Upstream precisa patch estreito para preservar garantias, não simples shell passthrough. Patches/lockfiles/origem entram na revisão da entrega, não fork invisível.
- Box sem plataforma nativa e com disco concorrido: nenhum gate Rust Linux repetido; provas disponíveis não substituem macOS/Windows. Run-owned targets/perfis e uma única prova Mac por vez, dados reais nunca fixture.
- Efeitos remotos irreversíveis e perda de resposta: registrar unknown, não retry em outro motor/modo ou após OAuth.
- OAuth do provedor pode exigir aceite externo. Entregar tudo alcançável, explicitar prerequisito sem mock/fallback enganoso; não considerar conexão pronta sem prova.
- Plano do dono mantém integração main antes do walkthrough Mac; qualquer falha local retorna à box e segue novo review/merge. Não declarar entrega final nem arquivar enquanto critérios nativos relevantes estiverem abertos.
