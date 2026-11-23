//! Turn bindings: which Clovy CLI chat turn an MCP connection belongs to.
//!
//! Before a CLI turn starts, the chat engine issues a token here
//! ([`TurnRegistry::issue`]) and passes it to `clovy-mcp` as `--turn
//! <token>`; the relay sends it in the handshake (`"turn"`), and the channel
//! binds the connection to the turn ([`TurnRegistry::bind`]). The token is
//! never a credential on its own: the handshake still needs the installation
//! secret and the same user. It lives as long as the run: revoking it (run
//! finished, failed, or cancelled; app quitting) closes every connection
//! bound to it, and a handshake offering it again is refused with
//! `turn_expired`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rand::RngCore;
use tokio::sync::watch;

use crate::agent_sessions::domain::AgentSafetyMode;
use crate::llm::cli::CliKind;

/// Random bytes in a token (hex on the wire: 64 characters).
pub const TOKEN_BYTES: usize = 32;
/// Handshake refusal: the turn token is unknown or revoked.
pub const TURN_EXPIRED: &str = "turn_expired";
/// Tool error: the tool exists only for connections bound to a turn.
pub const TURN_REQUIRED: &str = "turn_required";

/// The CLI turn a connection is bound to. Holds no token, so it can be
/// logged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnBinding {
    pub authority: crate::domain::turn_authority::TurnAuthority,
    /// Passive transport metadata shared with this turn's stream translator.
    /// Canonical tool identity and authority remain with the host.
    pub wire_names: Option<super::wire_names::WireNames>,
    pub session_id: String,
    pub run_id: String,
    /// Host-owned run snapshot, never the current view or a model argument.
    pub data_partition: String,
    pub cli: CliKind,
    pub safety_mode: AgentSafetyMode,
    /// A person watches the turn and can answer an approval card (`false`
    /// for routines: every approval is denied at once).
    pub attended: bool,
    /// Where the turn started (`chat`, `home`, `note_chat`, `companion`,
    /// `routine`); a free label for logs and the approval broker.
    pub surface: String,
}

struct Live {
    binding: Arc<TurnBinding>,
    /// Dropped on revocation, which wakes every bound connection.
    revoked: watch::Sender<()>,
}

/// The turns whose tokens are live. Cheap to clone (shared); one per app,
/// held by `McpServerState`, and kept across server restarts.
#[derive(Clone, Default)]
pub struct TurnRegistry {
    turns: Arc<Mutex<HashMap<String, Live>>>,
    on_revoke: Arc<Mutex<Vec<RevokeHook>>>,
}

type RevokeHook = Arc<dyn Fn(&str) + Send + Sync>;

impl std::fmt::Debug for TurnRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Tokens never reach logs.
        formatter
            .debug_struct("TurnRegistry")
            .field("live", &self.turns().len())
            .finish()
    }
}

/// A connection's live turn: the binding and a signal that fires when the
/// token is revoked.
pub struct BoundTurn {
    pub binding: Arc<TurnBinding>,
    alive: watch::Receiver<()>,
}

impl BoundTurn {
    /// Resolves once the token is revoked (immediately if it already is).
    pub async fn revoked(&mut self) {
        while self.alive.changed().await.is_ok() {}
    }
}

fn new_token() -> String {
    let mut bytes = [0u8; TOKEN_BYTES];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

impl TurnRegistry {
    /// Recheck the exact host binding after an approval or another wait.
    /// A matching database row alone does not keep a revoked turn alive.
    pub fn contains_binding(&self, binding: &TurnBinding) -> bool {
        self.turns().values().any(|live| *live.binding == *binding)
    }

    /// Revocation also cancels browser startup and in-flight control before
    /// the CLI connection is dropped. No tool/dispatch mutex is acquired.
    pub(crate) fn add_revoke_hook(&self, hook: RevokeHook) {
        self.on_revoke
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(hook);
    }

    fn notify_revoked(&self, run_id: &str) {
        let hooks = self
            .on_revoke
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        for hook in hooks {
            hook(run_id);
        }
    }
    fn turns(&self) -> MutexGuard<'_, HashMap<String, Live>> {
        self.turns.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Registers `binding` and returns its new token (64 hex characters).
    pub fn issue(&self, binding: TurnBinding) -> String {
        let live = Live {
            binding: Arc::new(binding),
            revoked: watch::channel(()).0,
        };
        let mut turns = self.turns();
        loop {
            let token = new_token();
            if !turns.contains_key(&token) {
                turns.insert(token.clone(), live);
                return token;
            }
        }
    }

    /// The live binding of `token`, if any.
    pub fn binding(&self, token: &str) -> Option<Arc<TurnBinding>> {
        self.turns()
            .get(token)
            .map(|live| Arc::clone(&live.binding))
    }

    /// Binds a connection to the live turn of `token`; `None` when the token
    /// is unknown or revoked (`turn_expired`).
    pub fn bind(&self, token: &str) -> Option<BoundTurn> {
        self.turns().get(token).map(|live| BoundTurn {
            binding: Arc::clone(&live.binding),
            alive: live.revoked.subscribe(),
        })
    }

    /// Revokes `token` and closes its connections. `false` when it was not
    /// live.
    pub fn revoke(&self, token: &str) -> bool {
        let live = self.turns().remove(token);
        if let Some(live) = live {
            live.binding.authority.revoke();
            self.notify_revoked(&live.binding.run_id);
            true
        } else {
            false
        }
    }

    /// Revokes every token of `run_id` (the run ended, failed, or was
    /// cancelled); returns how many.
    pub fn revoke_run(&self, run_id: &str) -> usize {
        let mut turns = self.turns();
        let before = turns.len();
        turns.retain(|_, live| {
            if live.binding.run_id == run_id {
                live.binding.authority.revoke();
                false
            } else {
                true
            }
        });
        let removed = before - turns.len();
        drop(turns);
        self.notify_revoked(run_id);
        removed
    }

    /// Revokes every token (the app is quitting).
    pub fn revoke_all(&self) {
        let runs: Vec<_> = self
            .turns()
            .drain()
            .map(|(_, live)| {
                live.binding.authority.revoke();
                live.binding.run_id.clone()
            })
            .collect();
        for run in runs {
            self.notify_revoked(&run);
        }
    }
}
