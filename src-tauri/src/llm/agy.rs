//! One app-owned Antigravity profile for chat, one-shot calls and native login.
//! Authentication belongs to the SDK. Only execution transport is disposable.
use super::agy_private::{self as private, Directory};
use crate::mcp_server::McpLaunch;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::File,
    io,
    path::{Path, PathBuf},
    sync::OnceLock,
};
use uuid::Uuid;

static HOST: OnceLock<HostContext> = OnceLock::new();
const CONFIG: &str = "mcp_config.json";
const RECEIPT: &str = ".clovy-mcp-transport.json";
const BOUNDARY_RECEIPT: &str = ".clovy-workspace-boundary.json";
const GIT_DIRECTORIES: &[&str] = &[
    ".git",
    ".git/objects",
    ".git/objects/info",
    ".git/objects/pack",
    ".git/refs",
    ".git/refs/heads",
    ".git/refs/tags",
    ".git/hooks",
    ".git/info",
];
const GIT_FILES: &[(&str, &[u8])] = &[
    (".git/HEAD", b"ref: refs/heads/main\n"),
    (
        ".git/config",
        b"[core]\nrepositoryformatversion = 0\nbare = false\nhooksPath = .git/hooks\n",
    ),
    (".git/info/exclude", b"/*\n"),
];

#[derive(Clone, Debug)]
pub struct HostContext {
    app_data: PathBuf,
    relay_env: BTreeMap<String, String>,
}
impl HostContext {
    pub fn new(app_data: PathBuf, host_env: &BTreeMap<String, String>) -> io::Result<Self> {
        let names = [
            "HOME",
            "USERPROFILE",
            "APPDATA",
            "LOCALAPPDATA",
            "HOMEDRIVE",
            "HOMEPATH",
        ];
        let relay_env: BTreeMap<_, _> = host_env
            .iter()
            .filter(|(key, _)| names.iter().any(|name| key.eq_ignore_ascii_case(name)))
            .map(|(key, value)| (key.to_ascii_uppercase(), value.clone()))
            .collect();
        let key = if cfg!(windows) { "APPDATA" } else { "HOME" };
        if !app_data.is_absolute()
            || !relay_env.iter().any(|(name, value)| {
                name.eq_ignore_ascii_case(key) && Path::new(value).is_absolute()
            })
        {
            return Err(private::denied());
        }
        Ok(Self {
            app_data,
            relay_env,
        })
    }
    /// Registered from app_paths::app_data_dir while the host HOME is intact.
    pub fn configure(self) -> io::Result<()> {
        if let Some(current) = HOST.get() {
            if current.app_data != self.app_data || current.relay_env != self.relay_env {
                return Err(private::denied());
            }
            return Ok(());
        }
        HOST.set(self).map_err(|_| private::denied())
    }
    pub fn current() -> io::Result<Self> {
        HOST.get().cloned().ok_or_else(|| {
            io::Error::other("Clovy has not resolved its Antigravity profile directory.")
        })
    }
    fn profile(&self) -> io::Result<Profile> {
        let at = |stage: &str, error: io::Error| {
            io::Error::new(error.kind(), format!("Antigravity {stage}: {error}"))
        };
        let app = Directory::root(&self.app_data).map_err(|error| at("app directory", error))?;
        let profiles = app
            .child("runtime-profiles")
            .map_err(|error| at("profiles directory", error))?;
        let root = profiles
            .child("agy")
            .map_err(|error| at("profile root", error))?;
        let home = root.child("home")?;
        // The existing run metadata stores this origin, not a second SDK
        // identity store. Replacing this home changes its filesystem identity.
        let origin = serde_json::to_vec(&(self.app_data.as_path(), home.identity()?))?;
        let id = format!("agy-profile-v1:{:x}", Sha256::digest(origin));
        // These are discovery roots, not sources of authentication data.
        for name in [".gemini", ".antigravity", ".agents"] {
            match std::fs::symlink_metadata(home.path().join(name)) {
                Ok(m) if m.file_type().is_symlink() || !m.is_dir() => return Err(private::denied()),
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
        let config = home.child(".config")?;
        let cache = home.child(".cache")?;
        let local = home.child(".local")?;
        let data = local.child("share")?;
        let state = local.child("state")?;
        let runtime = home.child(".runtime")?;
        let env = BTreeMap::from([
            ("HOME".into(), home.path().display().to_string()),
            (
                "XDG_CONFIG_HOME".into(),
                config.path().display().to_string(),
            ),
            ("XDG_CACHE_HOME".into(), cache.path().display().to_string()),
            ("XDG_DATA_HOME".into(), data.path().display().to_string()),
            ("XDG_STATE_HOME".into(), state.path().display().to_string()),
            (
                "XDG_RUNTIME_DIR".into(),
                runtime.path().display().to_string(),
            ),
        ]);
        #[cfg(windows)]
        let env = {
            let mut env = env;
            let appdata = home.child("appdata")?;
            env.insert("USERPROFILE".into(), home.path().display().to_string());
            let (drive, path) = windows_home_parts(home.path())?;
            env.insert("HOMEDRIVE".into(), drive);
            env.insert("HOMEPATH".into(), path);
            env.insert(
                "APPDATA".into(),
                appdata.child("roaming")?.path().display().to_string(),
            );
            env.insert(
                "LOCALAPPDATA".into(),
                appdata.child("local")?.path().display().to_string(),
            );
            env
        };
        Ok(Profile {
            root,
            home,
            id,
            env,
            relay_env: self.relay_env.clone(),
        })
    }
    pub fn chat(&self, session: &str) -> io::Result<Execution> {
        let profile = self.profile()?;
        // Stored session IDs need not be path components. Hashing is solely a
        // stable directory key, never a new conversation or memory identity.
        let key = format!("{:x}", Sha256::digest(session.as_bytes()));
        let directory = profile
            .root
            .child("workspaces")?
            .child("sessions")?
            .child(&key)?;
        Execution::new(profile, directory, false)
    }
    pub fn one_shot(&self) -> io::Result<Execution> {
        let profile = self.profile()?;
        let directory = profile
            .root
            .child("workspaces")?
            .child("calls")?
            .child(&Uuid::new_v4().simple().to_string())?;
        Execution::new(profile, directory, true)
    }
    pub fn login(&self) -> io::Result<Execution> {
        let profile = self.profile()?;
        let directory = profile
            .root
            .child("workspaces")?
            .child("login")?
            .child(&Uuid::new_v4().simple().to_string())?;
        // The interactive process outlives the app command. Its workspace is
        // durable until the terminal exits; it contains no turn authority.
        Execution::new(profile, directory, false)
    }
}

struct Profile {
    root: Directory,
    home: Directory,
    id: String,
    env: BTreeMap<String, String>,
    relay_env: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    version: u8,
    profile: String,
    server: String,
    identity: Vec<u64>,
    config: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    candidate: Option<String>,
}

// This receipt proves only ownership of the minimal discovery boundary. It is
// not SDK history/authentication and does not participate in cliProfileId.
#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct BoundaryProof {
    version: u8,
    workspace: Vec<u64>,
    directories: BTreeMap<String, Vec<u64>>,
    files: BTreeMap<String, Vec<u64>>,
}
struct Boundary {
    proof: BoundaryProof,
    receipt_identity: Vec<u64>,
    embedded: bool,
}

fn boundary_directory(workspace: &Directory, path: &str) -> io::Result<Directory> {
    boundary_directory_at(workspace, path, ".git")
}

fn boundary_directory_at(workspace: &Directory, path: &str, root: &str) -> io::Result<Directory> {
    let mut components = path.split('/');
    if components.next() != Some(".git") {
        return Err(private::denied());
    }
    let mut directory = workspace.existing_child(root)?;
    for component in components {
        directory = directory.existing_child(component)?;
    }
    Ok(directory)
}

impl Boundary {
    fn prepare(workspace: &Directory) -> io::Result<Self> {
        let boundary = if let Some((bytes, receipt_identity)) = workspace.read(BOUNDARY_RECEIPT)? {
            // Existing v1 boundaries remain valid without rewriting their proof.
            Self {
                proof: serde_json::from_slice(&bytes).map_err(|_| private::denied())?,
                receipt_identity,
                embedded: false,
            }
        } else if workspace.names()?.iter().any(|name| name == ".git") {
            let directory = workspace.existing_child(".git")?;
            let (bytes, receipt_identity) = directory
                .read(BOUNDARY_RECEIPT)?
                .ok_or_else(private::denied)?;
            Self {
                proof: serde_json::from_slice(&bytes).map_err(|_| private::denied())?,
                receipt_identity,
                embedded: true,
            }
        } else {
            if workspace.names()?.iter().any(|name| {
                name.to_str()
                    .is_some_and(|name| name.starts_with(".clovy-workspace-"))
            }) {
                tracing::warn!("Clovy retained unpublished Antigravity discovery staging without adopting it; initialization can retry");
            }
            let candidate = format!(".clovy-workspace-{}", Uuid::new_v4().simple());
            let directory = workspace.new_child(&candidate)?;
            #[cfg(test)]
            private::crash_point("boundary-root-created");
            let mut proof = BoundaryProof {
                version: 2,
                workspace: workspace.identity()?,
                directories: BTreeMap::from([(".git".into(), directory.identity()?)]),
                files: BTreeMap::new(),
            };
            for path in GIT_DIRECTORIES.iter().skip(1) {
                let (parent, name) = path.rsplit_once('/').ok_or_else(private::denied)?;
                let child =
                    boundary_directory_at(workspace, parent, &candidate)?.new_child(name)?;
                proof
                    .directories
                    .insert((*path).to_string(), child.identity()?);
            }
            #[cfg(test)]
            private::crash_point("boundary-directories-created");
            for (path, contents) in GIT_FILES {
                let (parent, name) = path.rsplit_once('/').ok_or_else(private::denied)?;
                let identity = boundary_directory_at(workspace, parent, &candidate)?
                    .write_new(name, contents)?;
                proof.files.insert((*path).to_string(), identity);
            }
            // Publish the complete proof inside its owned unit, then move the
            // whole unit into .git without replacement. No final name can show
            // a partially initialized boundary or receipt.
            let receipt_identity = directory.publish_new(
                BOUNDARY_RECEIPT,
                &serde_json::to_vec(&proof)?,
                "boundary-receipt",
            )?;
            #[cfg(test)]
            private::crash_point("boundary-receipt-published");
            let boundary = Self {
                proof,
                receipt_identity,
                embedded: true,
            };
            boundary.validate_at(workspace, &candidate)?;
            drop(directory);
            workspace.publish_directory(&candidate, ".git", &boundary.proof.directories[".git"])?;
            boundary
        };
        boundary.validate(workspace)?;
        #[cfg(test)]
        private::crash_point("boundary-published");
        Ok(boundary)
    }

    fn validate(&self, workspace: &Directory) -> io::Result<()> {
        self.validate_at(workspace, ".git")
    }

    fn validate_at(&self, workspace: &Directory, root: &str) -> io::Result<()> {
        workspace.validate()?;
        let (bytes, identity) = if self.embedded {
            if workspace.read(BOUNDARY_RECEIPT)?.is_some() {
                return Err(private::denied());
            }
            boundary_directory_at(workspace, ".git", root)?
                .read(BOUNDARY_RECEIPT)?
                .ok_or_else(private::denied)?
        } else {
            workspace
                .read(BOUNDARY_RECEIPT)?
                .ok_or_else(private::denied)?
        };
        if identity != self.receipt_identity
            || serde_json::from_slice::<BoundaryProof>(&bytes)
                .ok()
                .as_ref()
                != Some(&self.proof)
            || self.proof.version != if self.embedded { 2 } else { 1 }
            || self.proof.workspace != workspace.identity()?
            || self.proof.directories.len() != GIT_DIRECTORIES.len()
            || self.proof.files.len() != GIT_FILES.len()
        {
            return Err(private::denied());
        }
        for path in GIT_DIRECTORIES {
            let directory = boundary_directory_at(workspace, path, root)?;
            if self.proof.directories.get(*path) != Some(&directory.identity()?) {
                return Err(private::denied());
            }
            let mut expected: Vec<_> = GIT_DIRECTORIES
                .iter()
                .copied()
                .chain(GIT_FILES.iter().map(|(path, _)| *path))
                .filter_map(|child| {
                    child
                        .rsplit_once('/')
                        .filter(|(parent, _)| parent == path)
                        .map(|(_, name)| name)
                })
                .collect();
            if self.embedded && *path == ".git" {
                expected.push(BOUNDARY_RECEIPT);
            }
            let names = directory.names()?;
            if names.len() != expected.len()
                || names
                    .iter()
                    .any(|name| !expected.iter().any(|expected| name == expected))
            {
                return Err(private::denied());
            }
        }
        for (path, contents) in GIT_FILES {
            let (parent, name) = path.rsplit_once('/').ok_or_else(private::denied)?;
            let (bytes, identity) = boundary_directory_at(workspace, parent, root)?
                .read(name)?
                .ok_or_else(private::denied)?;
            if bytes != *contents || self.proof.files.get(*path) != Some(&identity) {
                return Err(private::denied());
            }
        }
        Ok(())
    }

    fn discard(&self, workspace: &Directory, lease: &[u64]) -> io::Result<()> {
        self.validate(workspace)?;
        let names = workspace.names()?;
        let mut allowed = vec![".git", ".agents", ".clovy-turn.lock"];
        if !self.embedded {
            allowed.push(BOUNDARY_RECEIPT);
        }
        if names
            .iter()
            .any(|name| !allowed.iter().any(|allowed| name == allowed))
        {
            return Err(private::denied());
        }
        let agents = workspace.existing_child(".agents")?;
        if !agents.names()?.is_empty() {
            return Err(private::denied());
        }
        let agents_identity = agents.identity()?;
        drop(agents);
        // Validate every unit before removing any of them. All operations are
        // nonrecursive and reject symlinks, changed identity and unknown data.
        let file = workspace.file(".clovy-turn.lock", false)?;
        if private::identity(&file)? != lease {
            return Err(private::denied());
        }
        drop(file);
        for (path, _) in GIT_FILES {
            let (parent, name) = path.rsplit_once('/').ok_or_else(private::denied)?;
            boundary_directory(workspace, parent)?.remove(name, &self.proof.files[*path])?;
        }
        for path in GIT_DIRECTORIES.iter().rev() {
            let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
            if parent.is_empty() {
                if self.embedded {
                    boundary_directory(workspace, ".git")?
                        .remove(BOUNDARY_RECEIPT, &self.receipt_identity)?;
                }
                workspace.remove_child(name, &self.proof.directories[*path])?;
            } else {
                boundary_directory(workspace, parent)?
                    .remove_child(name, &self.proof.directories[*path])?;
            }
        }
        if !self.embedded {
            workspace.remove(BOUNDARY_RECEIPT, &self.receipt_identity)?;
        }
        workspace.remove_child(".agents", &agents_identity)?;
        workspace.remove(".clovy-turn.lock", lease)
    }
}

pub struct Execution {
    profile: Profile,
    directory: Option<Directory>,
    lease: Option<File>,
    boundary: Boundary,
    disposable: bool,
    server: Option<String>,
}
impl Execution {
    fn new(profile: Profile, directory: Directory, disposable: bool) -> io::Result<Self> {
        let lease = private::lease(&directory)?;
        let boundary = Boundary::prepare(&directory).map_err(|error|io::Error::new(error.kind(),format!("Could not seal Antigravity workspace discovery; its files were preserved: {error}")))?;
        let mut execution = Self {
            profile,
            directory: Some(directory),
            lease: Some(lease),
            boundary,
            disposable,
            server: None,
        };
        execution.finish()?; // recover a previous crashed turn under this lease
        Ok(execution)
    }
    pub fn cwd(&self) -> &Path {
        self.directory.as_ref().expect("live execution").path()
    }
    pub fn profile_id(&self) -> &str {
        &self.profile.id
    }
    pub fn home(&self) -> &Path {
        self.profile.home.path()
    }
    pub fn server(&self) -> Option<&str> {
        self.server.as_deref()
    }
    pub fn settings(&self) -> String {
        let mut deny = vec![
            "command(*)",
            "read_file(*)",
            "write_file(*)",
            "read_url(*)",
            "execute_url(*)",
            "unsandboxed(*)",
        ];
        let allow: Vec<_> = match self.server() {
            Some(server) => vec![format!("mcp({server}/*)")],
            None => {
                deny.push("mcp(*)");
                Vec::new()
            }
        };
        json!({"toolPermission":"request-review","useG1Credits":false,"modelProvider":"",
            "permissions":{"allow":allow,"ask":[],"deny":deny},
            "statusLine":{"enabled":false},"title":{"enabled":false}})
        .to_string()
    }
    /// Keep OS/process necessities, not user API billing or configuration
    /// overrides. Native OAuth remains the SDK's own persistent mechanism.
    pub fn environment(&self, base: &BTreeMap<String, String>) -> BTreeMap<String, String> {
        let mut env: BTreeMap<_, _> = base
            .iter()
            .filter(|(name, _)| {
                let name = name.to_ascii_uppercase();
                matches!(
                    name.as_str(),
                    "PATH"
                        | "PATHEXT"
                        | "SYSTEMROOT"
                        | "WINDIR"
                        | "COMSPEC"
                        | "TEMP"
                        | "TMP"
                        | "TMPDIR"
                        | "LANG"
                        | "TERM"
                        | "COLORTERM"
                        | "TZ"
                        | "NO_COLOR"
                        | "DO_NOT_TRACK"
                ) || name.starts_with("LC_")
            })
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        env.extend(self.profile.env.clone());
        env
    }
    pub fn validate(&self) -> io::Result<()> {
        self.profile.root.validate()?;
        self.profile.home.validate()?;
        self.boundary
            .validate(self.directory.as_ref().ok_or_else(private::denied)?)
    }
    pub fn install_mcp(&mut self, launch: &McpLaunch) -> io::Result<()> {
        self.validate()?;
        if !launch.command.is_absolute() {
            return Err(private::denied());
        }
        let channel = launch
            .args
            .iter()
            .position(|arg| arg == "--dir")
            .and_then(|i| launch.args.get(i + 1))
            .ok_or_else(private::denied)?;
        if !Path::new(channel).is_absolute() {
            return Err(private::denied());
        }
        let agents = self
            .directory
            .as_ref()
            .ok_or_else(private::denied)?
            .child(".agents")?;
        if agents.read(CONFIG)?.is_some() || agents.read(RECEIPT)?.is_some() {
            return Err(private::denied());
        }
        // The SDK qualifies tools as mcp_<server>_<tool>, capped at 64 bytes.
        // Keep the full nonce in 23 bytes; this transport name is not authority.
        let mut server = String::with_capacity(23);
        server.push('c');
        URL_SAFE_NO_PAD.encode_string(Uuid::new_v4().as_bytes(), &mut server);
        let config = json!({"mcpServers":{(server.clone()):{"command":launch.command,"args":launch.args,"timeoutSeconds":900,
            "env":self.profile.relay_env}}});
        let (candidate, identity) = agents.stage("config", &serde_json::to_vec_pretty(&config)?)?;
        let receipt = Receipt {
            version: 3,
            profile: self.profile.id.clone(),
            server: server.clone(),
            identity,
            config: config.clone(),
            candidate: Some(candidate.clone()),
        };
        // Both final names are published atomically from complete private
        // candidates. The receipt proves ownership before CONFIG is visible.
        agents.publish_new(RECEIPT, &serde_json::to_vec(&receipt)?, "receipt")?;
        #[cfg(test)]
        private::crash_point("receipt-published");
        agents.publish(&candidate, CONFIG, &receipt.identity)?;
        #[cfg(test)]
        private::crash_point("config-published");
        self.server = Some(server);
        Ok(())
    }
    /// The caller revokes its TurnGrant BEFORE invoking transport cleanup.
    /// No code here traverses or removes native authentication state.
    pub fn finish(&mut self) -> io::Result<()> {
        self.finish_transport().and_then(|()|self.boundary.validate(self.directory.as_ref().ok_or_else(private::denied)?))
            .map_err(|error|io::Error::new(error.kind(),format!("Antigravity execution MCP configuration or discovery boundary needs recovery; its private files were retained: {error}")))
    }
    fn finish_transport(&mut self) -> io::Result<()> {
        let agents = self
            .directory
            .as_ref()
            .ok_or_else(private::denied)?
            .child(".agents")?;
        let Some((bytes, receipt_identity)) = agents.read(RECEIPT)? else {
            if agents.read(CONFIG)?.is_some() {
                return Err(private::denied());
            }
            if agents.names()?.iter().any(|name| {
                name.to_str().is_some_and(|name| {
                    name.starts_with(".clovy-mcp-config-")
                        || name.starts_with(".clovy-mcp-receipt-")
                })
            }) {
                tracing::warn!("Clovy retained unpublished Antigravity MCP candidates without an ownership receipt; the session can retry");
            }
            return Ok(());
        };
        let receipt: Receipt = serde_json::from_slice(&bytes).map_err(|_| private::denied())?;
        let candidate_valid = match (&receipt.candidate, receipt.version) {
            (None, 1) => true,
            (Some(name), 2 | 3) => name
                .strip_prefix(".clovy-mcp-config-")
                .is_some_and(|id| Uuid::parse_str(id).is_ok()),
            _ => false,
        };
        let server_valid = match receipt.version {
            // Existing durable receipts remain cleanup-only recovery input.
            1 | 2 => receipt
                .server
                .strip_prefix("clovy_turn_")
                .is_some_and(|id| Uuid::parse_str(id).is_ok()),
            3 => receipt.server.strip_prefix('c').is_some_and(|nonce| {
                nonce.len() == 22
                    && URL_SAFE_NO_PAD
                        .decode_slice(nonce, &mut [0; 16])
                        .is_ok_and(|len| len == 16)
            }),
            _ => false,
        };
        if !candidate_valid
            || receipt.profile != self.profile.id
            || !server_valid
            || receipt.config["mcpServers"]
                .as_object()
                .map_or(true, |servers| {
                    servers.len() != 1 || !servers.contains_key(&receipt.server)
                })
        {
            return Err(private::denied());
        }
        let config = agents.read(CONFIG)?;
        if let Some((config, identity)) = &config {
            // v1 wrote this identified unit in place. A prefix of its exact
            // expected serialization is an interrupted owned write, not proof
            // permitting arbitrary malformed or foreign content to be removed.
            let legacy_partial = receipt.version == 1
                && serde_json::to_vec_pretty(&receipt.config)?.starts_with(config);
            if identity != &receipt.identity
                || (!legacy_partial
                    && serde_json::from_slice::<Value>(config).ok().as_ref()
                        != Some(&receipt.config))
            {
                return Err(private::denied());
            }
        }
        let candidate = if let Some(name) = &receipt.candidate {
            agents.read(name)?
        } else {
            None
        };
        if let Some((bytes, identity)) = &candidate {
            if identity != &receipt.identity
                || serde_json::from_slice::<Value>(bytes).ok().as_ref() != Some(&receipt.config)
            {
                return Err(private::denied());
            }
        }
        // Validate both names before removing either; a foreign replacement
        // stays retained. Caller revocation precedes every cleanup operation.
        if let Some((_, identity)) = config {
            agents.remove(CONFIG, &identity)?;
        }
        if let (Some(name), Some((_, identity))) = (&receipt.candidate, candidate) {
            agents.remove(name, &identity)?;
        }
        agents.remove(RECEIPT, &receipt_identity)?;
        self.server = None;
        Ok(())
    }
}
impl Drop for Execution {
    fn drop(&mut self) {
        if self.finish().is_err() {
            tracing::error!(
                "Clovy Antigravity transport cleanup needs recovery; private receipt retained"
            );
            return;
        }
        if self.disposable {
            let discarded = (|| -> io::Result<()> {
                let directory = self.directory.as_ref().ok_or_else(private::denied)?;
                let lease = private::identity(self.lease.as_ref().ok_or_else(private::denied)?)?;
                self.boundary.discard(directory, &lease)?;
                let path = directory.path().to_path_buf();
                let identity = directory.identity()?;
                // Close the lease and directory before rmdir on Windows. A
                // one-shot has an exclusive UUID path, never another turn's cwd.
                drop(self.lease.take());
                drop(self.directory.take());
                let parent = Directory::root(path.parent().ok_or_else(private::denied)?)?;
                parent.remove_child(
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .ok_or_else(private::denied)?,
                    &identity,
                )
            })();
            if discarded.is_err() {
                tracing::warn!(
                    "Clovy retained an Antigravity one-shot workspace with unknown or changed data"
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "agy_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "agy_discovery_tests.rs"]
mod discovery_tests;

#[cfg(test)]
#[path = "agy_boundary_tests.rs"]
mod boundary_tests;

#[cfg(windows)]
fn windows_home_parts(home: &Path) -> io::Result<(String, String)> {
    use std::path::{Component, Prefix};
    let Some(Component::Prefix(prefix)) = home.components().next() else {
        return Err(private::denied());
    };
    let drive = match prefix.kind() {
        Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => format!("{}:", char::from(letter)),
        Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => format!(
            "\\\\{}\\{}",
            server.to_string_lossy(),
            share.to_string_lossy()
        ),
        _ => return Err(private::denied()),
    };
    let path = home
        .strip_prefix(prefix.as_os_str())
        .map_err(|_| private::denied())?
        .to_string_lossy()
        .replace('/', "\\");
    if !path.starts_with('\\') {
        return Err(private::denied());
    }
    Ok((drive, path))
}
