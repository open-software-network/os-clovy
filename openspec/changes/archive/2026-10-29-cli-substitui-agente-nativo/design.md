# Design

## Contratos (congelados)

Congelados em 2026-10-15 pelo Main (tarefa 0.8) sobre `cli-20261015/phase0-engine` (0.2-0.7 e os ajustes do congelamento). Mudança de contrato a partir daqui volta ao Main e é registrada nesta seção. Caminhos relativos a `src-tauri/src/`.

### C4, grupos de ferramentas (`mcp_server/tools/mod.rs`, `mcp_server/protocol.rs`)

```rust
pub struct McpData { notes, profile, memory_enabled, activity, now }   // inalterado; sem AppHandle
#[derive(Clone, Copy)]
pub struct ToolCtx<'a> {
    pub data: &'a McpData,
    pub app: Option<&'a AppHandle>,        // None sem runtime Tauri (testes)
    pub binding: Option<&'a TurnBinding>,  // None = cliente sem vínculo
    pub broker: Option<&'a ApprovalBroker>,// o do app (mod.rs) ou o de teste
}
impl<'a> ToolCtx<'a> {
    pub fn unbound(data: &'a McpData) -> ToolCtx<'a>;                       // sem app, vínculo nem broker
    pub fn broker(&self) -> Result<&'a ApprovalBroker, AppError>;           // approval_unavailable se None
}
pub trait ToolGroup: Send + Sync {
    fn needs_turn(&self) -> bool { true }  // só ReadTools devolve false
    fn owns(&self, name: &str) -> bool;    // decide "Unknown tool" x turn_required
    fn descriptors(&self, ctx: &ToolCtx<'_>) -> Vec<Value>;
    fn call<'a>(&'a self, ctx: &'a ToolCtx<'a>, name: &'a str, arguments: &'a Value)
        -> BoxFuture<'a, Result<Value, AppError>>;
}
pub fn groups() -> Vec<&'static dyn ToolGroup>;  // registro: uma linha por grupo (Main)
pub fn descriptors(ctx: &ToolCtx<'_>) -> Vec<Value>;
pub fn is_known_tool(name: &str) -> bool;
pub async fn call(ctx: &ToolCtx<'_>, name: &str, arguments: &Value) -> Result<Value, AppError>;
// protocol.rs
pub async fn handle(data: &McpData, message: &Value) -> Option<Value>;    // = handle_in(&ToolCtx::unbound(data), …)
pub async fn handle_in(ctx: &ToolCtx<'_>, message: &Value) -> Option<Value>;
pub fn tool_result(value: &Value) -> Value;   // {content:[{type:text,text:<json>}], isError:false}
pub fn tool_error(message: &str) -> Value;    // isError:true
```

- Grupo com `needs_turn() == true`: ausente de `tools/list` sem vínculo; `tools/call` sem vínculo devolve `isError` terminado em `(turn_required)` sem executar o grupo.
- `Err(AppError)` de `ToolGroup::call` vira `isError` com texto `"<message> (<code>)"` (`protocol.rs`). O código é a parte estável para o modelo e para os testes.
- `ToolGroup::call` devolve `Result` (não `Option<Result>` como esboçado em §1): `owns` separa "não é meu" de "falhou". O `AppHandle` fica em `ToolCtx`, não em `McpData` (desvio de §1), para os testes montarem `McpData` sem runtime Tauri.
- **Adendo 2026-10-15, blocos de conteúdo (aditivo, autorizado pelo Main na revisão de 2.6).** Uma ferramenta pode devolver blocos de conteúdo MCP em vez de um texto JSON:

  ```rust
  // protocol.rs
  pub const CONTENT_KEY: &str = "$mcpContent";
  pub fn content_value(blocks: Vec<Value>) -> Value;           // {"$mcpContent": [...]}
  pub fn text_block(text: impl Into<String>) -> Value;         // {type:"text", text}
  pub fn image_block(data: impl Into<String>, mime_type: impl Into<String>) -> Value; // {type:"image", data:<base64>, mimeType:"image/*"}
  ```

  `tool_result` devolve `{content: <blocos>, isError: false}` quando o valor é exatamente `{"$mcpContent": [...]}` com blocos `text`/`image` bem formados; qualquer outro valor (inclusive blocos inválidos ou vazios) continua num único bloco de texto JSON, então as ferramentas existentes não mudam. Usam hoje: `computer_use` (captura como `image`, texto e conteúdo estruturado como `text`) e `screenshot` do navegador (PNG como `image` e o resultado do broker como `text`). Imagens acima de 3 MiB em base64 ficam no Clovy. Test: `computer_use_tests::content_values_become_mcp_blocks_and_plain_values_stay_one_text_block` e `tests/mcp_server_stdio.rs::mcp_server_binary_carries_image_content_blocks_to_the_cli` (binário `clovy-mcp`).
- **Adendo 2026-10-15, catálogo assíncrono (aditivo, Main, desbloqueio de 2.9/2.10).** `ToolGroup` ganha `fn catalog<'a>(&'a self, ctx: &'a ToolCtx<'a>) -> BoxFuture<'a, Vec<Value>>`, cujo padrão devolve `descriptors(ctx)`; `tools::descriptors(ctx)` passa a ser `async` e `tools/list` aguarda o `catalog` de cada grupo visível, na ordem de registro. Grupos estáticos continuam implementando só `descriptors`. Grupos cujo catálogo depende de I/O (contas de conectores, inventário do Notion hospedado, descoberta de MCPs do usuário e Linear) sobrescrevem `catalog`, reutilizando a descoberta assíncrona existente; falha de descoberta de um servidor omite as ferramentas dele e não derruba o `tools/list`. `owns(name)` continua síncrono: grupos dinâmicos decidem por prefixo (`mcp_`, `notion-`) ou por um nome fixo da tabela do delta.
- **Broker no contexto.** `mcp_server/mod.rs` preenche `broker` com `app.try_state::<ApprovalBroker>()`; testes passam o de `approval_broker::testing::TestBroker` (abaixo). Um grupo de escrita nunca age sem `ctx.broker()?` aprovar: sem broker a ferramenta falha com `approval_unavailable`.

### C1, vínculo de turno (`mcp_server/{turns.rs,channel.rs,stdio.rs,mod.rs}`, `chat_engine/{turn.rs,mod.rs}`)

```rust
// mcp_server/turns.rs
pub const TOKEN_BYTES: usize = 32;                 // token = 64 hex
pub const TURN_EXPIRED: &str = "turn_expired";     // recusa de handshake
pub const TURN_REQUIRED: &str = "turn_required";   // erro de ferramenta
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnBinding {                           // sem o token: pode ir para log
    pub session_id: String, pub run_id: String, pub cli: CliKind,
    pub safety_mode: AgentSafetyMode, pub attended: bool,
    pub surface: String,                           // "chat" (assistido) | "routine" (não assistido) hoje
}
#[derive(Clone, Default)] pub struct TurnRegistry;  // Arc interno; Debug só com a contagem
impl TurnRegistry {
    pub fn issue(&self, binding: TurnBinding) -> String;           // token novo
    pub fn binding(&self, token: &str) -> Option<Arc<TurnBinding>>;
    pub fn bind(&self, token: &str) -> Option<BoundTurn>;          // usado pelo canal
    pub fn revoke(&self, token: &str) -> bool;                     // fecha as conexões do token
    pub fn revoke_run(&self, run_id: &str) -> usize;               // fim/falha/cancelamento
    pub fn revoke_all(&self);                                      // shutdown (shutdown.rs)
}
pub struct BoundTurn { pub binding: Arc<TurnBinding>, /* sinal */ }
impl BoundTurn { pub async fn revoked(&mut self) }
// mcp_server/mod.rs
impl McpServerState { pub fn turns(&self) -> &TurnRegistry }    // um registro por app, sobrevive a religar o servidor
impl McpLaunch { pub fn for_turn(&self, token: &str) -> McpLaunch } // acrescenta ["--turn", token]
// mcp_server/channel.rs
pub type TurnHandler = Arc<dyn Fn(Value, Option<Arc<TurnBinding>>) -> Pin<Box<dyn Future<Output = Option<Value>> + Send>> + Send + Sync>;
pub struct Service;  // From<Handler> (legado, ignora o vínculo) e From<TurnHandler>
impl Listener {
    pub async fn start(dir: &Path, handler: impl Into<Service>) -> io::Result<Self>;  // registro vazio
    pub async fn start_with_turns(dir: &Path, handler: impl Into<Service>, turns: TurnRegistry) -> io::Result<Self>;
}
pub fn turn_handshake(secret: &str, turn: Option<&str>) -> Value;
pub fn connect_turn(dir: &Path, turn: Option<&str>) -> Result<UnixStream, ClientError>;  // ClientError::TurnExpired
// mcp_server/stdio.rs
pub fn parse_turn(args: &[String]) -> Result<Option<String>, String>;  // "--turn" vazio = erro de uso (saída 2)
pub fn relay_turn(dir: &Path, turn: Option<&str>, input: impl BufRead, output: Box<dyn Write + Send>);
pub const TURN_EXPIRED_MESSAGE: &str;  // "... (turn_expired)"

// chat_engine/turn.rs (lado do motor, 0.4)
pub struct TurnContext {                 // um turno resolvido
    /* session_id, run_id, kind, program, env, workspace, input, */
    pub mcp: Option<McpLaunch>,          // servidor sem vínculo; o turno acrescenta --turn
    pub turns: Option<TurnRegistry>,     // None = servidor sem vínculo (só leitura)
    pub safety_mode: AgentSafetyMode,
    pub attended: bool,                  // chat true, rotinas false
    pub images: Vec<PathBuf>,            // anexos de imagem (caminhos absolutos no workspace)
}
impl TurnContext { pub fn permissions(&self) -> TurnPermissions; pub fn binding(&self) -> TurnBinding }
pub struct TurnGrant;                    // token do turno; Drop = revoke
impl TurnGrant { pub fn issue(turns: &TurnRegistry, binding: TurnBinding) -> Self; pub fn token(&self) -> &str }
// chat_engine/mod.rs
pub struct CliTurnRequest { /* session_id, run_id, kind, workspace, input, */
    pub images: Vec<PathBuf>, pub safety_mode: AgentSafetyMode, pub attended: bool }
pub fn check_policy(kind: CliKind, attended: bool) -> Result<(), AppError>;  // agent_cli_unattended_unsupported
pub async fn cancel_cli_run(host: &ChatEngineHost, broker: Option<&ApprovalBroker>,
    turns: Option<&TurnRegistry>, run_id: &str) -> bool;
```

- Fio: handshake `{"clovyMcp": 1, "secret": "<hex>", "turn": "<token>"}`; `turn` ausente ou `null` = conexão sem vínculo; string desconhecida/revogada ou valor não string = `{"clovyMcp": 1, "ok": false, "error": "turn_expired"}`. O token só é consultado depois que segredo e uid conferem. Vale para várias conexões enquanto a execução vive (o relay reconecta); um token por execução, nunca reaproveitado. Revogar fecha só as conexões daquele token: a conexão para de ler pedidos, entrega as respostas das chamadas em voo (no máximo 2 s, `REVOKE_GRACE` em `channel.rs`) e fecha; chamadas novas, e uma chamada ainda sem resposta no fim do prazo, recebem `TURN_EXPIRED_MESSAGE` do relay (conexão vinculada que cai com o socket de pé = turno revogado). Relay antigo sem `--turn` ou app antigo que ignora `turn` = conexão sem vínculo.
- Emissão: `execute_turn` emite o token quando o CLI aceita MCP e há `turns` (`start_cli_turn` pega `McpServerState::turns()`), monta `launch.for_turn(token)` (Claude e Copilot no arquivo temporário, Codex em `-c mcp_servers.clovy.args`) e segura o `TurnGrant` até o processo terminar; todo retorno revoga. O token não vai no prompt nem no ambiente.
- Cancelamento: `cancel_agent_run` chama `cancel_cli_run`: `broker.cancel_run(run)` (cartões `cancelled`), `turns.revoke_run(run)`, depois `ChatEngineHost::cancel` (SIGTERM ao grupo). A chamada que esperava o cartão responde `run_cancelled` ao CLI antes de a conexão revogada fechar. O ouvinte de eventos terminais do broker continua cobrindo fim e falha.
- Encerramento: `ChatEngineHost::shutdown` cancela cada turno e espera até 5 s (cada `TurnGrant` revoga ao terminar); em seguida `shutdown::run_cleanup` chama `McpServerState::turns().revoke_all()`.

### C2, broker de aprovação (`approval_broker/`)

```rust
pub const APPROVAL_TIMEOUT: Duration = Duration::from_secs(600); // < CLI_TOOL_TIMEOUT (900 s), checado em compilação
pub const CLI_ENGINE: &str = "cli";                              // `engine` do item

pub struct ApprovalTurn { pub session_id: String, pub run_id: String, pub cli: String,
    pub safety_mode: AgentSafetyMode, pub attended: bool }
impl From<&TurnBinding> for ApprovalTurn;                        // ctx.binding → turno do broker
pub struct ApprovalRequest { pub tool: String, pub call_id: Option<String>, pub kind: ApprovalKind }
pub enum ApprovalKind {
    Approval { title: String, description: String, arguments: Value, preview: Option<String> },
    Clarification { question: String, choices: Vec<String> },
}
pub enum ApprovalOutcome { Approved, Answered(String) }
pub enum ApprovalDenial { UserDenied, TimedOut, RunCancelled, UnattendedRequiresApproval, Unavailable(String) }
impl ApprovalDenial { pub fn code(&self) -> &'static str; pub fn message(&self) -> String }

impl ApprovalBroker {               // estado Tauri (`app.state::<ApprovalBroker>()`), clonável
    pub fn new(io: impl BrokerIo, clock: impl Clock, timeout: Duration) -> Self;  // testes: banco em memória + relógio manual
    pub async fn request(&self, turn: &ApprovalTurn, request: ApprovalRequest)
        -> Result<ApprovalOutcome, ApprovalDenial>;
    pub async fn cancel_run(&self, run_id: &str) -> usize;   // pendentes da execução → run_cancelled
    pub fn close(&self);                                     // encerramento do app
    pub fn waiting_ids(&self) -> Vec<String>;
}
// approval_broker/testing.rs (#[cfg(test)]): broker para testes de grupos, sem Tauri
pub(crate) enum Scripted { Approve, Deny, Answer(String), Leave }   // decisão do próximo cartão
impl TestBroker {
    pub async fn new(script: impl IntoIterator<Item = Scripted>) -> Self;  // banco em memória com migrações
    pub async fn with_timeout(script, timeout: Duration) -> Self;
    pub fn broker(&self) -> &ApprovalBroker;                 // para ToolCtx.broker
    pub fn pool(&self) -> &SqlitePool;
    pub async fn turn(&self, cli: CliKind, attended: bool) -> TurnBinding;  // sessão + execução running
    pub fn requests(&self) -> Vec<Value>;                    // cartões pedidos, em ordem
    pub fn events(&self) -> Vec<(String, Value)>;
}
pub trait BrokerIo: Send + Sync + 'static { fn pool(&self) -> BoxFuture<'_, Result<SqlitePool, AppError>>;
    fn emit(&self, session_id: &str, run_id: &str, sequence: i64, method: &str, data: Value); }
// approval_broker/commands.rs
pub async fn resolve_cli_interruption(app: &AppHandle, run_id: &str, interruption_id: &str, resolution: &Value)
    -> Result<Option<AgentRunDto>, AppError>;   // Ok(None) = não é item CLI
```

| `ApprovalDenial` | `code()` (erro da ferramenta, `isError`) | Item |
|---|---|---|
| `UserDenied` | `user_denied` | `resolved`, `resolution: "deny"` |
| `TimedOut` | `approval_timed_out` | `expired`, `retiredReason: "timeout"` |
| `RunCancelled` | `run_cancelled` | `cancelled`, `retiredReason: "cancelled"` (ou `"restarted"` na abertura) |
| `UnattendedRequiresApproval` | `unattended_requires_approval` | nenhum (sem cartão) |
| `Unavailable(_)` | `approval_unavailable` | falha de armazenamento; a ferramenta não executa |

- **Item.** Mesmo `interruption` do nativo (`id`, `sessionId`, `runId`, `status`, `createdAt`, `kind`, `toolName`, `toolCallId`, `title`, `description`, `command`, `allowAlways: false` ou `question`/`choices`) com `engine: "cli"` e `cli`. `title`/`description` vêm do código da ferramenta, nunca do modelo; `command` é gerado pelo broker de `preview` ou dos argumentos, com `sanitize_log`. `external_id` = `interruption:{run}:{id}`.
- **Eventos.** `interruption.requested` ao pedir e `interruption.updated` (mesmo `data: {itemId, interruption}`) ao decidir, expirar ou cancelar. O broker não avança `agent_runs.last_sequence`; o frontend aplica eventos de interrupção CLI fora do portão de sequência (`src/lib/agent-runtime-adapter.ts`).
- **Resolução.** `resolve_agent_interruption` chama `resolve_cli_interruption` primeiro; item com `engine: "cli"` nunca chega a `run.resume`. `{"kind":"approval","choice":"once"|"session"|"always"}` aprova uma vez (gravado `resolution: "once"`), `"deny"` nega; clarificação exige `{"kind":"clarification","answer"}`. Segunda resposta, resposta tardia ou de outro processo → `agent_interruption_expired`; tipo trocado → `agent_interruption_resolution_invalid`.
- **Decisão única.** Quem remove o waiter da tabela em memória é dono do desfecho (decisão, timeout, cancelamento, queda da chamada MCP); o item só sai de `pending` uma vez (`UPDATE … WHERE status = 'pending'`). Sem pendentes, a execução volta de `waiting_for_user` a `running`.
- **Fim da execução.** `setup` escuta `run.completed`/`run.cancelled`/`run.failed` e chama `cancel_run`; `cancel_cli_run` cancela antes de parar o processo. `close()` roda em `RunEvent::ExitRequested`. Na abertura, `store::cancel_stale` marca `cancelled` os itens CLI pendentes de outro processo e encerra a execução como `cancelled`.
- **Companion (D4).** O telefone recebe `waitingForUser` pelo evento `interruption.requested` e um aviso de que o pedido aguarda resposta no Mac. Nenhuma resposta remota para itens CLI nesta fase; `computer_use` pelo telefone é a fatia 2.6.

### C3, política de permissões (`chat_engine/policy.rs`, consumida por `chat_engine/cli.rs`)

```rust
pub const CLI_TOOL_TIMEOUT: Duration = Duration::from_secs(900);  // claude MCP_TOOL_TIMEOUT, codex tool_timeout_sec (S3)
pub const CLAUDE_INTERACTION_TOOLS: bool = false;   // 2.11 liga: --permission-prompt-tool + --disallowedTools
pub const CLAUDE_PROMPT_TOOL: &str = "mcp__clovy__approve_action";
pub const FORBIDDEN: &[&str];                       // fragmentos proibidos em qualquer argv
#[derive(Clone, Copy)] pub struct TurnPermissions { pub safety_mode: AgentSafetyMode, pub attended: bool }
pub struct TurnPolicy { pub args: Vec<String>, pub mcp_args: Vec<String>, pub env: Vec<(&'static str, String)> }
pub fn turn_policy(kind: CliKind, permissions: TurnPermissions) -> TurnPolicy;
pub fn image_args(kind: CliKind, images: &[PathBuf]) -> Vec<String>;   // codex: --image por anexo
pub fn forbidden_in(arg: &str) -> Option<&'static str>;
// cli.rs
pub fn turn_invocation(kind, prompt, conversation, mcp: Option<&McpLaunch>, scratch,
    permissions: TurnPermissions, images: &[PathBuf]) -> TurnInvocation;  // + set_env
```

| Turno | Claude Code | Codex |
|---|---|---|
| Assistido, `sandboxed` | `--permission-mode default`; com o servidor do Clovy `--allowedTools mcp__clovy__*`; env `MCP_TOOL_TIMEOUT=900000` | `-c sandbox_mode="workspace-write"`, `-c sandbox_workspace_write.network_access=false`, `-c approval_policy="never"`, `-c web_search="live"` |
| Assistido, `unrestricted` (D1) | `--permission-mode acceptEdits` (edições no workspace; shell e o resto continuam perguntando), resto igual | igual, com `network_access=true` (escrita ainda só no workspace) |
| Não assistido (rotinas) | `--permission-mode default --tools Read,Grep,Glob,WebSearch,WebFetch --allowedTools Read,Grep,Glob,WebSearch,WebFetch,mcp__clovy__* --strict-mcp-config` (fase 1, revisada) | `-c sandbox_mode="read-only"`, `-c approval_policy="never"`, `-c web_search="live"` |
| Com o servidor do Clovy, todo turno Codex | n/a | `-c mcp_servers.clovy.default_tools_approval_mode="approve"` (S2, só o servidor `clovy`) e `-c mcp_servers.clovy.tool_timeout_sec=900` |
| Quando `CLAUDE_INTERACTION_TOOLS` (2.11) | `--permission-prompt-tool mcp__clovy__approve_action --disallowedTools mcp__clovy__approve_action AskUserQuestion` (S1) | n/a |

- pi, agy, cursor-agent e copilot não têm linha: modo não interativo padrão, só turnos assistidos (`check_policy`).
- Prompt sempre por stdin no Claude e no Codex (S1: listas variádicas). No Codex, `--image` vem antes dos `-c` (o `--image` de `exec` também é variádico); caminho com vírgula fica só no manifesto. Claude lê imagens pelo manifesto.
- Turnos assistidos nunca levam `--strict-mcp-config` (D7). `--allowedTools` em turno assistido só `mcp__clovy__*`; a única exceção é o conjunto só-leitura das rotinas.
- Proibidos em qualquer combinação (`FORBIDDEN`, teste de varredura): `--dangerously-skip-permissions`, `--allow-dangerously-skip-permissions`, `bypassPermissions`, `--dangerously-bypass-approvals-and-sandbox`, `danger-full-access`, `--approve-for-me`, `--yolo`, `--full-auto`, `--always-approve`, `--allow-all`, `--allow-all-tools`, `--approve-mcps`.

### Exemplo mínimo: grupo de escrita com broker e vínculo

Versões executáveis: `ProbeWrite` em `chat_engine/binding_tests.rs` (direto com `TestBroker` e ponta a ponta pelo socket e relay).

```rust
pub struct NotesWrite;
pub const SAVE_THING: &str = "save_thing";

impl ToolGroup for NotesWrite {
    fn owns(&self, name: &str) -> bool { name == SAVE_THING }
    fn descriptors(&self, _ctx: &ToolCtx<'_>) -> Vec<Value> {
        vec![json!({ "name": SAVE_THING, "description": "…", "inputSchema": { "type": "object" } })]
    }
    fn call<'a>(&'a self, ctx: &'a ToolCtx<'a>, name: &'a str, arguments: &'a Value)
        -> BoxFuture<'a, Result<Value, AppError>> {
        Box::pin(async move {
            // needs_turn() == true: tools::call só chega aqui com vínculo.
            let binding = ctx.binding.ok_or_else(|| AppError::new(TURN_REQUIRED, "…"))?;
            let request = ApprovalRequest {
                tool: name.to_string(),
                call_id: None,
                kind: ApprovalKind::Approval {
                    title: "Save a thing".into(),                 // texto do app, nunca do modelo
                    description: "Clovy saves one thing in this session.".into(),
                    arguments: arguments.clone(),
                    preview: None,
                },
            };
            ctx.broker()?
                .request(&ApprovalTurn::from(binding), request)
                .await
                .map_err(|denial| AppError::new(denial.code(), denial.message()))?;
            // Só depois da aprovação: escrever, usando binding.session_id/run_id.
            Ok(json!({ "saved": true }))
        })
    }
}
// tools/mod.rs (Main): groups.push(&notes::NotesWrite);
```

Teste da fatia sem socket (o caminho padrão da fase 2):

```rust
let broker = TestBroker::new([Scripted::Approve, Scripted::Deny]).await;
let binding = broker.turn(CliKind::Claude, true).await;
let ctx = ToolCtx { data: &data, app: None, binding: Some(&binding), broker: Some(broker.broker()) };
assert!(NotesWrite.call(&ctx, SAVE_THING, &args).await.is_ok());                          // aprovado
assert_eq!(NotesWrite.call(&ctx, SAVE_THING, &args).await.unwrap_err().code, "user_denied");
let routine = broker.turn(CliKind::Codex, false).await;                                      // sem cartão
assert_eq!(NotesWrite.call(&ToolCtx { binding: Some(&routine), ..ctx }, SAVE_THING, &args)
    .await.unwrap_err().code, "unattended_requires_approval");
```

Ponta a ponta (quando a fatia precisar do fio): `TurnRegistry` + `Listener::start_with_turns` + `stdio::relay_turn` com o token, como `chat_engine/binding_tests.rs`.

## Context

Estado em `main` @ ba23ae95 (todas as citações relativas à raiz do repositório):

- **Turno CLI.** Um processo por mensagem, argv montado em `turn_invocation` (`src-tauri/src/chat_engine/cli.rs:84-220`). Claude: `-p --output-format stream-json --verbose --include-partial-messages [--resume <id>] --mcp-config <scratch>/clovy-mcp.json` (`cli.rs:95-120`). Codex: `exec [resume] --json --skip-git-repo-check -c sandbox_mode="workspace-write" -c mcp_servers.clovy.command=… -c mcp_servers.clovy.args=… [<id>] -` (`cli.rs:121-150`). Só Claude, Codex e Copilot aceitam MCP (`cli.rs:61-63`). Nenhuma flag que pule ou pré-conceda aprovação é passada (`cli.rs:18-22`, `docs/llm-providers.md` seção "CLI chat engine"); recusas viram `tool.failed` com `needsApproval` (`src-tauri/src/chat_engine/turn.rs:453-474`).
- **Sem modo de segurança no CLI.** `AgentSafetyMode {Sandboxed, Unrestricted}` (`src-tauri/src/agent_runtime/domain.rs:4-27`) é gravado na sessão (`src-tauri/src/agent_runtime/api.rs:690-697`), mas `CliTurnRequest` (`src-tauri/src/chat_engine/mod.rs:168-175`) e `TurnContext` (`turn.rs:36-47`) não o carregam.
- **MCP do Clovy.** Relay stdio → socket Unix 0600 com segredo da instalação e checagem de uid (`src-tauri/src/mcp_server/channel.rs:51-67,241-245`; ADR-0058). Handshake `{clovyMcp:1, secret}` sem identidade de sessão (`channel.rs:51-53`). Protocolo sem estado (`src-tauri/src/mcp_server/protocol.rs:1-4`); o app nunca envia requisições ao cliente (`protocol.rs:43-44`). Ferramentas só de leitura (`src-tauri/src/mcp_server/tools.rs:122-129`, despacho em `tools.rs:295-325`). Limites: 8 conexões, 4 requisições em voo por conexão (`channel.rs:30-35`); nenhum timeout por requisição no app nem no relay (`channel.rs:410`, `src-tauri/src/mcp_server/stdio.rs:220-249`). `McpData` não carrega `AppHandle` (`src-tauri/src/mcp_server/mod.rs:316-346`).
- **Aprovações nativas.** `requiresApproval` vira `needsApproval` do SDK (`agent-runtime/src/sdk-engine.ts:252`); a interrupção é persistida como item `interruption` e a execução vai a `waiting_for_user` (`src-tauri/src/agent_runtime/host.rs:1061-1107`); o frontend recebe `clovy://agent-runtime-event` (`host.rs:25,952`) e responde com `resolve_agent_interruption` (`api.rs:1268-1276`, núcleo `api.rs:1335-1735`). Cartões: `ApprovalPart`, `ClarifyPart` (`src/components/agent/chat-turns/AgentActionCards.tsx:15,337`), `SecretPart` (`src/components/agent/chat-turns/TurnPresentation.tsx:27-90`). Sem timeout no desktop; "session"/"always" não viram concessão persistida (`api.rs:1440-1452`). Companion só aprova `computer_use`, uma vez, com prazo de 60 s após o recibo (`src-tauri/src/companion/mod.rs:714-728,1282-1450`; ADR-0053).

## Goals / Non-Goals

**Goals:** paridade funcional dos turnos Claude Code/Codex com o agente nativo, com o mesmo limite de autoridade (credenciais no app, aprovação humana para escrita, política do host para MCP do usuário); um único broker de aprovação reutilizado por todas as ferramentas de escrita; remoção limpa do nativo depois da paridade comprovada.

**Non-Goals:** fase 1 (padrão de motor, roteamento, rotinas só-leitura; worktree `cli-20261015/cli-routing`); dar ferramentas do Clovy a pi, agy e cursor-agent; editar a configuração global dos CLIs; remover rotas do clovy-api; mudar a ADR-0058 para clientes externos.

## Inventário de paridade

Legenda da coluna "Destino": **CLI** = o Claude Code/Codex já cobre sozinho; **MCP** = ferramenta nova no `clovy-mcp` (nomes fixados no delta `clovy-mcp-server`); **Gateway** = repasse de servidor MCP pelo `clovy-mcp`. "Aprov." = passa pelo broker.

| Capacidade nativa | Onde está hoje | Destino | Aprov. | Observação |
|---|---|---|---|---|
| `search_june`, `get_note`, `get_dictation` | `api.rs:2367-2369`; `src-tauri/src/agent_runtime/tools.rs:430-579` | MCP: `search_notes` e `list_dictations` já existem (`mcp_server/tools.rs:160-184`); **novo** `get_dictation` (por id, paginado) e argumento aditivo `contentOffset` em `get_note` | não | Nativo pagina conteúdo e transcrição por bytes (256 KiB, `tools.rs:19,482-534`); MCP corta o conteúdo em 20.000 caracteres (`mcp_server/tools.rs:453-495`). Paridade = paginar conteúdo por caractere, mantendo a retenção de ditados do MCP (`dictation_history_cutoff_timestamp`). |
| `list_memories`, `save_memory`, `forget_memory` | `api.rs:2370-2372`; `tools.rs:619-694` | MCP: `list_memories` existe; **novos** `save_memory`, `forget_memory` | sim (escrita) | Mesmo gate de memória global e por projeto (`tools.rs:973-1002`). |
| `generate_image`, `edit_image`, `generate_video` | `api.rs:2373-2375`; `tools.rs:696-798`; clovy-api `/v1/image/*`, `/v1/video/*` (`src-tauri/src/clovy_api.rs:876-964`) | MCP: **novos**, mesmos nomes | não (paridade) | Artefato gravado no workspace da sessão e em `agent_artifacts` (`tools.rs:774-788,1181-1194`): exige vínculo de turno. Vídeo faz polling longo; depende do timeout de ferramenta do CLI (spike S3). |
| `start_recording`, `stop_recording`, `recording_status` | `api.rs:2377-2379`; `tools.rs:160-189`; `src-tauri/src/agent_recorder.rs:53-100` (ADR-0013) | MCP: **novos**, mesmos nomes | start/stop sim | O broker aprova e depois o `AgentRecorderBroker` existente executa (espera até 420 s, `agent_recorder.rs:12`). |
| Browser use (14) | `api.rs:2380-2393`; `tools.rs:230-258`; `src-tauri/src/browser_broker.rs:1108-1136` | MCP: **novos**, mesmos 14 nomes | aprovação própria do broker do navegador | Ações consequentes já estacionam na fila do `BrowserBroker` (`browser_broker.rs:1652-1670`, 600 s) com a bandeja existente (`browser_approvals_pending`/`browser_approval_respond`, `browser_broker.rs:1935-1953`); o broker de aprovação não duplica isso. Contexto `Attended` ligado ao turno. |
| `computer_use` | `api.rs:2414`; `tools.rs:220-226`; `src-tauri/src/computer_use.rs:2601-2610` | MCP: **novo** `computer_use` | sim, a cada chamada | Exige lease de execução assistida (`computer_use.rs:2298-2315,2809-2820`) agora concedido pelo vínculo de turno; permissão por app continua no broker do computer use (`computer_use.rs:3648-3690`). Companion: escopo ADR-0053 (D4). |
| `get_obsidian_vault` | `api.rs:2376`; `tools.rs:156-159`; `src-tauri/src/obsidian.rs:91-110` | MCP: **novo** `get_obsidian_vault`; leitura dos arquivos pelo próprio CLI | não | Leitura fora do workspace passa pelo prompt de permissão do CLI (Claude → broker; Codex `workspace-write` já lê fora). |
| `list_skills`, `load_skill` | `api.rs:2404-2405`; `tools.rs:1483-1554,1686-1696` | MCP: **novos**, mesmos nomes | não | Nativo filtra pelo snapshot da execução (`agent_runs.enabled_skills_json`); no MCP filtra pelas habilitações globais (`agent_skill_settings`). Skills do próprio CLI continuam do CLI (D9). |
| Rotinas CRUD | `api.rs:2406-2411`; `tools.rs:202-214,278-298`; `src-tauri/src/routines.rs:192-368` | MCP: **novos** `list_routines`, `create_routine`, `update_routine`, `pause_routine`, `resume_routine`, `delete_routine` | todas menos `list_routines` | A execução da rotina em CLI é da fase 1 (`routines.rs:741-814` hoje usa `run.start`). |
| Gmail, Calendar, GitHub | `src-tauri/src/agent_runtime/native_connectors.rs:38-151,229-283,515-586`; tokens em `src-tauri/src/connectors/store.rs:1-23` | MCP: **novos**, mesmos nomes do catálogo nativo, com `accountId` | mutações sim | Tokens nunca saem do app; revalidação de escopo por chamada como hoje (`native_connectors.rs:546-586`). |
| Notion (MCP hospedado) | `src-tauri/src/connectors/notion.rs:32-67,271-312`; `tools.rs:1416-1481` | MCP: repasse dos nomes externos `notion-*` pelo `clovy-mcp` | ações (`notion-create-pages`, `notion-update-page`) sim | Hoje a ação exige vínculo de aprovação com digest por `run_id/call_id` (`tools.rs:1435-1475`); passa a exigir o id de aprovação do broker. |
| Linear (MCP hospedado, ADR-0050) | `src-tauri/src/agent_mcp.rs:65-67,1281-1340,1765-1839` | Gateway: `mcp_linear_*` | conforme anotações (`agent_mcp.rs:1313-1314`) | Bearer fica em memória no app (`agent_mcp.rs:1784-1839`). |
| MCPs do usuário (ADR-0039) | `agent_mcp.rs:35-53,194-228,357-417,1396-1423` | Gateway: `mcp_<servidor>_<tool>` | política do servidor (padrão: sim) | Segredos no Keychain `co.opensoftware.clovy.agent-mcp`; Seatbelt para stdio em sessão `sandboxed` (`agent_mcp.rs:1991-2003,2400-2420`). Alternativa: repasse direto (D2). |
| `request_clarification` | `api.rs:2412`; `tools.rs:1556,1587-1617` | MCP: **novo** `request_clarification` | broker (tipo clarificação) | A resposta volta como resultado da ferramenta; some o "pool" de respostas por execução. Claude: desabilitar `AskUserQuestion` no `-p` (spike S1). |
| `request_secret` | `api.rs:2413`; `tools.rs:1560-1585`; consumido só por `run_shell` `secretEnv` (`tools.rs:1330-1359`) | **não portado** (D5) | n/a | Sem consumidor seguro: o shell do CLI não aceita referência opaca e devolver o valor colocaria o segredo no contexto do modelo. |
| Arquivos (`list/read/write/patch/import/preview/search_files`), `run_shell` | `api.rs:2396-2403`; `tools.rs:1047-1414` | CLI | prompts do próprio CLI (Claude → broker via S1; Codex → sandbox) | `import_file` vira cópia pelo próprio CLI. |
| `web_search`, `web_fetch` | `api.rs:2394-2395`; `tools.rs:1024-1045` → clovy-api `/v1/web/*` | CLI (`WebSearch`/`WebFetch` do Claude; busca web do Codex, S5) | não | Desktop deixa de chamar `/v1/web/*`; rota fica no servidor (ADR-0021). |
| Atividade, dia | `src-tauri/src/activity/timeline/agent_tools.rs:29-60` | MCP: já existem (`docs/mcp-server.md` tabela) | não | Sem mudança. |
| Anexos e visão | `api.rs:2259-2297,2795-2915`; CLI recebe só manifesto de texto (`api.rs:890`) | CLI: Codex `--image <path>` por anexo de imagem; Claude lê a imagem pelo `Read` a partir do manifesto | não | Texto do manifesto deixa de citar "Clovy's file tools" (`api.rs:2900-2915`). |
| Compactação, steering | `api.rs:257-269,955-978` | CLI gerencia o próprio contexto; steering continua como fila de follow-up | n/a | Já recusados para CLI hoje. |

## Decisions

### 1. Vínculo de turno (contrato compartilhado C1)

- `chat_engine` cria, antes de lançar o CLI, um **token de turno** aleatório (32 bytes, hex) e registra em `TurnRegistry` (novo `src-tauri/src/mcp_server/turns.rs`) o `TurnBinding { token, session_id, run_id, cli, safety_mode, attended, surface, expires_on_run_end }`.
- O token vai nos argumentos do servidor: `["--dir", "<mcp dir>", "--turn", "<token>"]` no arquivo temporário do Claude (`cli.rs:106-114`) e no `-c mcp_servers.clovy.args=[…]` do Codex (`cli.rs:132-143`). Fica visível em `ps` para o mesmo usuário, que já lê o segredo da instalação (`docs/mcp-server.md`, "Local channel"); o token sozinho não abre o socket e morre com a execução.
- Handshake aditivo: `{"clovyMcp": 1, "secret": "<hex>", "turn": "<token>"}`. Versão continua 1; relay antigo sem `turn` = conexão não vinculada. Token desconhecido ou expirado → `{"ok": false, "error": "turn_expired"}`, e o relay responde as chamadas com esse motivo.
- `ChatEngineHost` revoga o token quando a execução termina, falha ou é cancelada (mesmo ponto que `register`/`shutdown`, `src-tauri/src/chat_engine/mod.rs:86-164`). Conexões vinculadas são fechadas na revogação.
- **Conexão não vinculada (clientes externos, Cursor, `claude mcp add`)**: `tools/list` e `tools/call` iguais aos de hoje (só leitura). Ferramentas de escrita nunca são listadas sem vínculo; chamá-las responde `turn_required`.
- `McpData` ganha `AppHandle` e `Option<TurnBinding>`; `tools.rs` vira `tools/mod.rs` com registro de **grupos** (`trait ToolGroup { fn descriptors(&ToolCtx) -> Vec<Value>; async fn call(&ToolCtx, name, args) -> Option<Result<Value>> }`), um arquivo por grupo, para fatias paralelas com donos disjuntos.

### 2. Broker de aprovação (contrato compartilhado C2)

Novo módulo `src-tauri/src/approval_broker/` (`mod.rs`, `store.rs`, `commands.rs`), independente do runtime nativo:

1. Uma ferramenta de escrita chama `broker.request(&binding, ApprovalRequest { kind: Approval | Clarification, tool, title, description, preview, target })`.
2. O broker grava um item `interruption` na sessão (mesmo `AgentItemPayload` que o nativo, id `interruption:{run}:{uuid}`, `engine: "cli"`), põe a execução em `waiting_for_user` e emite `interruption.requested` em `clovy://agent-runtime-event` pelo mesmo `persist_and_emit_event` (`host.rs:888`) que o motor CLI já usa (`chat_engine/mod.rs:60-79`). Os cartões existentes renderizam sem novo componente.
3. A chamada MCP espera um `oneshot` com timeout (`APPROVAL_TIMEOUT` padrão 10 min; sempre menor que o timeout de ferramenta passado ao CLI, ver §3). O semáforo de 4 requisições por conexão (`channel.rs:304-314`) continua valendo; a espera ocupa uma vaga.
4. A UI responde com `resolve_agent_interruption` (mesmo comando): quando o item tem `engine: "cli"`, o comando entrega a decisão ao broker em vez de `run.resume`. Escolhas: `once` aprova, `deny` nega; `session`/`always` são tratados como `once` (como hoje, `api.rs:1440-1452`). Clarificação devolve `{answer}`.
5. Resultado: aprovado → a ferramenta executa e devolve seu resultado; negado → `isError` com `user_denied`; timeout → `approval_timed_out` e item marcado `expired`; execução cancelada ou app encerrando → `run_cancelled`, item `cancelled`. Uma decisão é aceita uma única vez (claim atômico do item, como `api.rs:1161-1230`).
6. **Sem humano**: binding com `attended = false` (rotinas) nega imediatamente com `unattended_requires_approval`, sem gravar cartão pendente (D6).
7. **Companion**: o evento de pendência é publicado ao telefone como hoje; só `computer_use` é respondível remotamente, com permissão de uso único e prazo de 60 s (ADR-0053 inalterada; ampliar é D4). As demais aparecem no telefone como "aguardando aprovação no Mac".
8. **Prompts do próprio Claude Code**: com `--permission-prompt-tool mcp__clovy__approve_action` (spike S1), o Claude envia cada pedido de permissão (Bash, Edit, Write, leitura fora do workspace) à ferramenta `approve_action`, que chama o mesmo broker e devolve `{"behavior":"allow","updatedInput":…}` ou `{"behavior":"deny","message":…}`. `approve_action` só é listada em conexão vinculada a turno Claude e é excluída do uso direto pelo modelo se o spike confirmar que `--disallowedTools` não bloqueia o caminho de permissão.
9. Na inicialização, interrupções CLI ainda `pending` (app fechado no meio) são marcadas `cancelled` e a execução `cancelled`; o CLI já morreu com o grupo de processos (`docs/llm-providers.md`, "App shutdown").

### 3. Política de permissões (contrato compartilhado C3)

Novo `src-tauri/src/chat_engine/policy.rs` produz flags a partir de `(cli, safety_mode, attended)`; `CliTurnRequest` e `TurnContext` ganham `safety_mode` e `attended`. Tabela original abaixo; a vigente, com os ajustes de S1-S5 e D1, está em "Contratos (congelados)", C3.

| Caso | Claude Code | Codex |
|---|---|---|
| Assistido, `sandboxed` (padrão) | `--permission-mode default` (spike S1 confirma o nome; a ajuda lista `manual`), `--permission-prompt-tool mcp__clovy__approve_action`, `--allowedTools mcp__clovy__*` (a autoridade das ferramentas do Clovy é o broker dentro delas; sem prompt duplo), `--disallowedTools AskUserQuestion`; env `MCP_TOOL_TIMEOUT` = 15 min | `-c sandbox_mode="workspace-write"` (como hoje), `-c approval_policy="never"`, `-c mcp_servers.clovy.tool_timeout_sec=900` |
| Assistido, `unrestricted` | igual, com `--permission-mode acceptEdits` (edições no workspace sem perguntar; shell e o resto continuam no broker) | igual, com `-c sandbox_workspace_write.network_access=true` (rede liberada, escrita ainda restrita ao workspace) |
| Não assistido (rotinas, fase 1) | política só-leitura da fase 1; com vínculo `attended=false` o broker nega toda escrita | `-s read-only` da fase 1; idem |

Nunca: `--dangerously-skip-permissions`, `--permission-mode bypassPermissions`, `--allow-dangerously-skip-permissions`, `--dangerously-bypass-approvals-and-sandbox`, `danger-full-access`, `--approve-for-me`. A regra "nunca `--permission-mode`/`--allowedTools`" de `docs/llm-providers.md` muda para "nunca flags que pulem aprovação; `--allowedTools` só para `mcp__clovy__*`". `--strict-mcp-config` não é passado: os MCPs que o usuário configurou dentro do próprio CLI continuam carregando (D7). Codex não tem prompt-tool em `exec`; suas ações próprias ficam limitadas pelo sandbox e as do Clovy pelo broker.

### 4. Gateway de MCP (Linear e servidores do usuário)

- Um grupo `gateway` no `clovy-mcp` lista, para conexões vinculadas, as ferramentas de `agent_mcp` habilitadas (mesma visibilidade e nomes `mcp_<servidor>_<tool>`, `agent_mcp.rs:1396-1423`) e o Linear gerenciado (`mcp_linear_*`).
- A chamada usa o caminho existente de `agent_mcp` (descoberta, sessão Streamable HTTP, refresh de token, Seatbelt para stdio em sessão `sandboxed`, limite de saída e timeout). A aprovação do servidor/ferramenta (`requires_approval`, `approval_tools`, anotações) passa pelo broker.
- Elicitação MCP de servidor do usuário vira clarificação do broker (substitui `tools.rs:374-424`).
- Motivo de não usar `--mcp-config` direto: exigiria gravar headers, env e tokens OAuth do Keychain num arquivo lido pelo CLI, quebrando ADR-0039 ("never receives connector tokens or MCP credentials") e ADR-0050, e perderia visibilidade, aprovação e Seatbelt. Repasse direto fica como D2.

### 5. Remoção do nativo (fase 4)

**Critério de corte**: fases 0 e 2 concluídas; `make verify` verde; teste de paridade por ferramenta (delta `clovy-mcp-server`) verde para Claude e Codex com CLI falso; smoke nativo isolado (AGENTS.md, "Functional proof") com turno real de Claude e de Codex usando ao menos uma ferramenta de cada grupo de escrita, aprovação e negação; nenhum ponto de entrada do app ainda inicia `run.start` (fase 1 concluída); dono aprova o corte.

**Apagar** (levantamento com linhas no inventário de tarefas 4.x):

- Sidecar: `agent-runtime/`; `pnpm-workspace.yaml:6`; scripts `agent-runtime:*` em `package.json:14-16,28`; `scripts/build-agent-runtime.mjs`, `scripts/benchmark-agent-runtime.mjs`; `src-tauri/AgentRuntimeEntitlements.plist`; bloco de `scripts/tauri-before-dev.mjs:91-96`; prefixo do `beforeBuildCommand` em `src-tauri/tauri.conf.json:8`; recursos em `src-tauri/tauri.macos.conf.json:17-18` e `src-tauri/tauri.windows.conf.json:7-8`; `ensure_agent_runtime_placeholder` em `src-tauri/build.rs:26,337-368`; `.gitignore:33`; blocos em `scripts/build-signed-dmg.sh:166-176,237-255`, `scripts/build-windows.ps1`, `scripts/qualify-windows-build.ps1`, `scripts/verify-windows-artifacts.ps1`; passos de CI em `.github/workflows/promote-desktop.yml:450-512,521-522,568-602`, `rc-desktop-dmg.yml:321-356,364,418-436`, `production-desktop-windows.yml:212-268,284-287,332-333`.
- Rust: `AgentRuntimeHost` e o loop de RPC/proxy em `host.rs:53-887,1297-1343`; `tools.rs` (ferramentas nativas), `native_connectors.rs`, `secrets.rs`; em `api.rs`: ramos nativos de `start/cancel/retry_agent_run` (`api.rs:736-818,930-951`, cauda de `retry` após `:1039`), `compact_agent_session`, `steer_agent_run` (ou stub `cli_engine`), interrupções do SDK (`api.rs:1116-1268,1335-1735` exceto o caminho CLI do broker), `run_params`/`tool_descriptors`/`resumable_run_config`/`runtime_history` (`api.rs:2243-2526`), skills por execução (`api.rs:1964-2131`, salvo D9), `resolve_companion_computer_use_approval` nativo (`api.rs:1284-1333`); `.manage(AgentRuntimeHost)` em `src-tauri/src/lib.rs:474`; orçamento de shutdown em `src-tauri/src/shutdown.rs:19-32,391,398`; comandos mortos `suggest_agent_session_title`/`explain_agent_approval` (`src-tauri/src/commands.rs:930,1018`) e chamadores; constantes de modelo de agente em `clovy_api.rs:56-63`; `forward_web_request` (`clovy_api.rs:1480-1512`); testes de endpoint→sidecar em `src-tauri/src/chat_engine/tests.rs:960-1128`.
- Frontend: `ComposerModelPicker` (`src/components/agent/composer/ModelPicker.tsx`) e usos em `AgentWorkspace.tsx`, `AgentSessionBar.tsx`, `NoteChatPanel.tsx`; ramos `clovy`/`endpoint` de `src/lib/chat-engine.ts:45-86`; `thinking-level`, `agent-default-model`, `agent-model-selection` (com edições em quem importa), `upstream-provider-recovery`, `agent-session-modes` (sem importadores); bindings `compactSession`/`steerRun`/skills/`explainAgentApproval` em `src/lib/tauri.ts:90-138,1115-1120`; seção de skills em `AgentSettingsSection.tsx` (salvo D9); chave Tavily em `AppSettings.tsx` (D8); testes listados na tarefa 4.3.
- Configurações: `llm_usage.chat` deixa de aceitar `clovy`/`endpoint` (migração única para o CLI padrão da fase 1); `set_provider_model(Generation)` deixa de mexer em `llm_usage.chat` (`src-tauri/src/providers/mod.rs:899-907`).

**Manter**: `agent_runtime::{domain, repository, protocol}` e a persistência de eventos (`host.rs:888-1290` parte de persistência, `sanitize_log` usado por `commands.rs:1010`) movidos para um módulo de sessão neutro; `persona.rs` (o Início deixou de usar em 2026-10-17, ADR-0064; decidir na 4.2 se ainda tem usuário); `migration.rs` (import Hermes, inerte); `home_import.rs` (importação do histórico antigo do Início, ADR-0064); comandos de sessão, itens, anexos e artefatos (`api.rs:122-642,1773-1880,2608-2959`); `agent_mcp.rs` (gateway), `browser_broker.rs`, `computer_use.rs`, `agent_recorder.rs`, `routines.rs` (em CLI); `proxy_agent_chat_completions` enquanto a segurança de imagem usar; migrações SQL (ADR-0037: nunca apagar arquivos de migração); chave OpenRouter BYOK (notas e limpeza).

**clovy-api**: em 2026-10-28 o dono decidiu que o app nunca chama o Clovy API e que o servidor sai do repositório; a remoção das rotas e do crate é a change `remover-clovy-api`, que roda depois desta fase 4.

**Histórico**: sessões nativas continuam legíveis (itens e artefatos são agnósticos de motor). Próxima mensagem numa sessão nativa vai para o CLI padrão com o histórico prefixado (`turn.rs:616-657`). `retry_agent_run` de execução nativa mapeia para o CLI padrão. Execuções `waiting_for_user` nativas são encerradas como `cancelled` por migração. Segredos órfãos do serviço `co.opensoftware.clovy.agent-secrets` são apagados uma vez.

**ADRs**: rascunho ADR-0063 nesta change (broker e escrita no MCP). Na fase 4, ADR nova que supera ADR-0038, ADR-0040, ADR-0013, as partes de runtime de ADR-0039 (rotinas e MCP continuam, agora sobre CLI) e reancora ADR-0053 no broker; ADR-0058 recebe adendo apontando para a ADR-0063; `spec/mcp-tool-naming.md` passa a apontar `src-tauri/src/mcp_server/tools/` e o delta `clovy-mcp-server` como dono dos nomes.

## Spikes obrigatórios (fase 0, antes das fatias)

| Id | Pergunta | Se a resposta for "não" |
|---|---|---|
| S1 | Claude 2.1.x em `-p` aceita `--permission-prompt-tool mcp__clovy__approve_action` com o formato `{behavior, updatedInput|message}`; qual é o nome do modo padrão (`default` vs `manual`); `--disallowedTools` bloqueia chamada direta sem bloquear o prompt; `AskUserQuestion` pode ser desabilitado | Claude fica como Codex: ações próprias recusadas viram `needsApproval` como hoje; só ferramentas do Clovy passam pelo broker. Registrar no design. |
| S2 | Codex 0.16x em `exec` pede aprovação para ferramentas MCP (e falha) ou as executa; `approval_policy="never"` muda isso | Se falhar, usar a configuração por servidor que o spike encontrar; nunca bypass. |
| S3 | Timeout real de ferramenta MCP em Claude (`MCP_TOOL_TIMEOUT`) e Codex (`tool_timeout_sec`) e se um valor de 15 min é aceito | `APPROVAL_TIMEOUT` e o polling de vídeo ficam abaixo do limite medido; vídeo devolve erro claro se não couber. |
| S4 | Codex `exec --image <path>` com `resume` | Imagens entram só pelo manifesto. |
| S5 | Busca web do Codex em `exec` (flag ou `-c`) | Codex fica sem busca web; documentar. |

Resultados entram em `design.md` (seção "Resultados dos spikes") antes da fase 2.

## Resultados dos spikes

Medidos em 2026-10-15 com `claude` 2.1.301 e `codex-cli` 0.162.1, workspace temporário vazio e um servidor MCP falso stdio (`echo` com `readOnlyHint`, `slow_tool` sem anotação, `approve_action`). Comandos, saídas brutas e ajudas completas: `.tmp/verify/cli-design/spikes/README.md` e `raw/` (fora do git). Autenticação: Claude não autentica com `CLAUDE_CONFIG_DIR` vazio, então a credencial existente foi só lida (`--no-session-persistence --setting-sources project`, sem `mcp add`; o CLI pode ter atualizado contadores em `~/.claude.json`, não verificado); Codex rodou com `CODEX_HOME` temporário com cópia de `auth.json` (hash do original igual antes e depois, cópia apagada). Custo: 5 turnos Claude (`--model haiku`, cerca de US$ 0,009) e 7 Codex.

| Id | Resposta medida | Consequência / fallback |
|---|---|---|
| S1 | **Sim.** Em `-p`, `--permission-prompt-tool mcp__<srv>__approve_action` recebe cada pedido como `tools/call approve_action {tool_name, input, tool_use_id}` (ex.: `Bash` com `{command, description}`). Resultado em texto JSON `{"behavior":"allow","updatedInput":<input>}` executa; `{"behavior":"deny","message":…}` vira `tool_result` com `is_error` e entra em `permission_denials`. `--permission-mode default` é aceito (não aparece na ajuda; `manual` é alias, ambos registram `permissionMode:"default"`). `--disallowedTools mcp__<srv>__approve_action` tira a ferramenta do modelo (nem `ToolSearch` acha) sem bloquear o caminho de permissão. `--disallowedTools AskUserQuestion` aceito. `--allowedTools 'mcp__<srv>__*'` aceito: ferramentas do servidor rodam sem passar pelo prompt-tool. D7: com servidor do turno de mesmo nome que um MCP do usuário e sem `--strict-mcp-config`, só o do `--mcp-config` carrega. | Tabela §3 do Claude confirmada, com `--disallowedTools mcp__clovy__approve_action AskUserQuestion`. Ajustes para 0.7: `--allowedTools`/`--disallowedTools` são variádicos e engolem um prompt posicional seguinte, então o prompt continua por stdin; sem `--strict-mcp-config` os conectores claude.ai da conta também carregam no turno (aceito por D7, registrar em `docs/llm-providers.md`). O Claude 2.1.301 envia antes `server/discover` (protocolo `2026-07-28`) e cai para `initialize` quando recebe erro: o `clovy-mcp` já responde `Method not found`, sem mudança. |
| S2 | **Não, por padrão.** `codex exec` já roda com `approval_policy="never"`; ferramenta MCP sem `readOnlyHint` falha com "MCP tool call requires approval, but approval policy is never"; com `readOnlyHint` executa. | **Fallback escolhido (não é bypass):** `-c mcp_servers.clovy.default_tools_approval_mode="approve"` só para o servidor `clovy`, cuja autoridade é o broker dentro de cada ferramenta de escrita; medido: ferramenta sem anotação executa. Alternativa por ferramenta: `mcp_servers.clovy.tools.<nome>.approval_mode="approve"` (não exercitado). Valores do enum: `auto | prompt | writes | approve`. A linha Codex de §3 ganha essa flag em 0.7. |
| S3 | **Sim.** Claude respeita `MCP_TOOL_TIMEOUT` (5000 ms: ferramenta de 10 s falha com "timed out after 5s"; 900000 ms: ferramenta de 70 s conclui). Codex respeita `mcp_servers.<srv>.tool_timeout_sec` (5: falha "timed out awaiting tools/call after 5s"; 900: 70 s conclui, acima do padrão de 60 s). | Valores de 15 min aceitos; `APPROVAL_TIMEOUT` de 10 min fica abaixo. Não provado: espera real de 15 min e o padrão do Claude sem a variável. |
| S4 | **Sim.** `codex exec resume --json --skip-git-repo-check --image <png> <thread_id> -` retoma a mesma thread (mesmo `thread_id`) e vê a imagem. Em `resume`, `--image` leva um arquivo por ocorrência: repetir a flag por anexo. | `--image` por anexo de imagem em turnos novos e retomados (0.7). |
| S5 | **Sim, por configuração.** `--search` só existe no `codex` interativo (`exec` recusa); `codex exec -c web_search="live"` produz item `web_search`. Valores: `live | indexed | cached | disabled`. | 0.7 passa `-c web_search="live"` quando a busca web deve existir. Não medido: o padrão sem configuração. |

## Decisões do dono

Aceitas pelo dono em 2026-10-15: D1-D9 conforme as recomendações abaixo. O dono também autorizou seguir até a remoção (fase 4); o critério de corte de `native-agent-removal` continua valendo, então a fase 4 só começa com a fase 3 (gate e smoke) aprovada.

- **D1 - `unrestricted` nos CLIs.** Recomendo o mapeamento da tabela §3 (Claude `acceptEdits`, Codex rede liberada, escrita sempre restrita ao workspace e shell sempre aprovado). Alternativa mais permissiva (Codex `danger-full-access`) não recomendada.
- **D2 - Linear/MCPs do usuário.** Recomendo gateway pelo `clovy-mcp` (credenciais ficam no app, política e Seatbelt mantidos). Repasse direto no `--mcp-config` só seria aceitável para servidores stdio sem segredo, e mesmo assim perde aprovação e Seatbelt.
- **D3 - Motor "endpoint" (modelo OpenAI-compatível como motor de chat).** Ele roda no sidecar (`src-tauri/src/chat_engine/mod.rs:4-6,262-318`). Recomendo removê-lo do chat na fase 4; endpoints continuam para notas, limpeza e atividade.
- **D4 - Companion.** Recomendo manter o escopo da ADR-0053 (só `computer_use` aprovável pelo telefone, uso único); demais aprovações só no Mac.
- **D5 - `request_secret`.** Recomendo não portar; segredos ficam na configuração do próprio CLI ou do servidor MCP no Keychain.
- **D6 - Rotinas com escrita.** Recomendo negar toda escrita não assistida. Pré-autorização por rotina no momento de salvar fica para uma change futura.
- **D7 - MCPs configurados dentro do CLI.** Recomendo não passar `--strict-mcp-config` (Clovy não decide pelo CLI do usuário); se o usuário também tiver `clovy` registrado com `claude mcp add`, o `--mcp-config` do turno prevalece (verificar em S1).
- **D8 - Tavily BYOK.** Recomendo esconder a chave Tavily nas Configurações na fase 4 (sem consumidor desktop) e manter o valor salvo inofensivo.
- **D9 - Skills do Clovy.** Recomendo expor `list_skills`/`load_skill` (baixo custo, sem escrita) e manter a seção de skills; alternativa é aposentá-las.

## Risks / Trade-offs

- [Prompt-tool do Claude é flag pouco documentada] → spike S1 com fallback explícito; nenhuma fatia depende dele além de 2L.
- [Aprovação longa estoura timeout do CLI] → timeouts configurados por turno e `APPROVAL_TIMEOUT` menor; teste com relógio controlado.
- [Token de turno visível em `ps`] → mesmo usuário já lê o segredo; token de uso por execução, revogado no fim, inútil sem o segredo.
- [Prompt injection pedindo escrita] → toda escrita do Clovy passa pelo broker com descrição gerada pelo app (não pelo modelo) e prévia dos argumentos sanitizados (`host.rs:1358-1371`).
- [4 requisições por conexão com aprovações pendentes] → aceitável (uma aprovação por vez é o caso comum); se o CLI paralelizar mais, as demais esperam no buffer, sem perda.
- [Fase 1 em paralelo] → esta change não toca arquivos de roteamento; fatias 2L e 4.x rebaseiam sobre a fase 1 integrada.
- [Número da ADR] → a fase 1 deve registrar a ADR-0062; este rascunho usa 0063. Se a numeração colidir na integração, renumerar o rascunho.

## Migration Plan

Fase 0 (contratos e spikes) → fase 1 (outro worker, em curso) → fase 2 (fatias de paridade em paralelo) → fase 3 (prova de paridade e aceite do dono) → fase 4 (remoção). Rollback até a fase 3: desligar as ferramentas de escrita é remover o vínculo de turno (conexões voltam a ser só leitura); o nativo continua instalado até a fase 4.
