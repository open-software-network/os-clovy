//! Anonymous-profile recovery. All destructive operations stay relative to
//! retained directory handles. The lease excludes admitted writers; a root
//! record takes over exclusion before the last controls are removed.
use super::clovy_browser_grant::{boot_identity, lock_profile, Owned};
use super::clovy_browser_process_identity::process_definitely_exited;
#[cfg(unix)]
use super::clovy_browser_process_identity::unix_group_exited;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::{self, Read, Write},
    path::Path,
};

type Id = [u64; 2];
const RECEIPT: &str = ".clovy-reserved.json";
const LEASE: &str = ".clovy-lease";
const MARKER: &str = ".clovy-starting";
const HOLDER: &str = ".clovy-gc-";
const TERMINAL: &str = ".clovy-gc-complete-";
const CONTROLS: [&str; 3] = [RECEIPT, LEASE, MARKER];
const HOLDER_RECEIPT: &str = ".clovy-holder.json";
fn invalid() -> io::Error {
    io::Error::other("browser recovery identity or state changed")
}
fn canonical(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value)
}
fn read_json(file: &mut File) -> io::Result<Value> {
    read_json_bounded(file, 16384)
}
fn read_json_bounded(file: &mut File, limit: u64) -> io::Result<Value> {
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(invalid());
    }
    Ok(serde_json::from_slice(&bytes)?)
}
fn optional<T>(value: io::Result<T>) -> io::Result<Option<T>> {
    match value {
        Ok(v) => Ok(Some(v)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}
#[derive(Debug)]
struct Entry {
    name: OsString,
    id: Id,
    directory: bool,
    link: bool,
}

pub(crate) struct RecoveryRoot {
    directory: File,
}
pub(crate) struct RecoveryProfile {
    root: File,
    location: ProfileLocation,
    directory: File,
    root_id: Id,
    id: Id,
    nonce: String,
}
enum ProfileLocation {
    Original,
    #[cfg(unix)]
    UnixHolder(File),
    #[cfg(windows)]
    WindowsPinnedOriginal,
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum TerminalLocation {
    UnixHolder { holder: Id, holder_receipt: Id },
    WindowsPinnedOriginal,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Control {
    name: String,
    id: Id,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Terminal {
    version: u32,
    phase: String,
    nonce: String,
    root: Id,
    location: TerminalLocation,
    profile: Id,
    controls: Vec<Control>,
    receipt: Value,
}
#[derive(Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HolderReceipt {
    version: u32,
    nonce: String,
    root: Id,
    holder: Id,
    profile: Id,
}
// Construction requires the same physical locked lease and a fully parsed
// receipt. This value retains every control until the terminal record owns it.
struct Cleanup {
    profile: RecoveryProfile,
    lease: File,
    receipt_file: File,
    marker: Option<File>,
    receipt: Value,
}
impl RecoveryRoot {
    pub(crate) fn open(path: &Path) -> io::Result<Self> {
        Ok(Self {
            directory: native::root(path, false)?,
        })
    }
    pub(crate) fn open_or_create(path: &Path) -> io::Result<Self> {
        Ok(Self {
            directory: native::root(path, true)?,
        })
    }
    pub(crate) fn names(&self) -> io::Result<Vec<OsString>> {
        Ok(native::list(&self.directory)?
            .into_iter()
            .map(|e| e.name)
            .collect())
    }
    pub(crate) fn profile(&self, name: &OsStr) -> io::Result<RecoveryProfile> {
        let nonce = name
            .to_str()
            .filter(|s| canonical(s))
            .ok_or_else(invalid)?
            .to_owned();
        let directory = native::directory(&self.directory, name, false)?;
        self.captured(directory, ProfileLocation::Original, nonce)
    }
    fn captured(
        &self,
        directory: File,
        location: ProfileLocation,
        nonce: String,
    ) -> io::Result<RecoveryProfile> {
        Ok(RecoveryProfile {
            root: self.directory.try_clone()?,
            root_id: native::identity(&self.directory)?,
            id: native::identity(&directory)?,
            directory,
            location,
            nonce,
        })
    }
    pub(crate) fn create_profile(&self, name: &OsStr) -> io::Result<RecoveryProfile> {
        let nonce = name
            .to_str()
            .filter(|s| canonical(s))
            .ok_or_else(invalid)?
            .to_owned();
        let directory = native::directory(&self.directory, name, true)?;
        native::sync(&self.directory)?;
        self.captured(directory, ProfileLocation::Original, nonce)
    }
    #[cfg(unix)]
    fn quarantined(&self, nonce: &str) -> io::Result<RecoveryProfile> {
        let holder = native::directory(
            &self.directory,
            OsStr::new(&format!("{HOLDER}{nonce}")),
            false,
        )?;
        let directory = native::directory(&holder, OsStr::new("profile"), false)?;
        let profile = self.captured(
            directory,
            ProfileLocation::UnixHolder(holder),
            nonce.to_owned(),
        )?;
        profile.validate_links()?;
        Ok(profile)
    }
    /// Returns diagnostics rather than treating failed enumeration as empty.
    /// Unknown namespaces, legacy files, links and ambiguous owners are retained.
    pub(crate) fn recover(&self) -> io::Result<Vec<String>> {
        let names = self.names()?;
        let mut diagnostics = Vec::new();
        // Finalize terminal records first. Never recurse payload from these.
        for name in &names {
            let Some(name) = name.to_str() else { continue };
            if let Some(nonce) = name
                .strip_prefix(TERMINAL)
                .and_then(|s| s.strip_suffix(".json"))
                .filter(|s| canonical(s))
            {
                if let Err(error) = self.finish_terminal(nonce) {
                    diagnostics.push(format!("terminal {nonce}: {error}"));
                }
            }
        }
        for name in names {
            let Some(name) = name.to_str() else { continue };
            let nonce = if canonical(name) {
                Some(name)
            } else {
                name.strip_prefix(HOLDER).filter(|s| canonical(s))
            };
            let Some(nonce) = nonce else { continue };
            // A terminal record is sole authority once controls may be absent.
            // Any error other than absence also excludes the nonterminal path.
            if !matches!(native::file(&self.directory, OsStr::new(&terminal_name(nonce)), false, false), Err(e) if e.kind() == io::ErrorKind::NotFound)
            {
                continue;
            }
            let result = (|| {
                let candidate = if canonical(name) {
                    self.profile(OsStr::new(name))
                } else {
                    #[cfg(unix)]
                    {
                        self.quarantined(nonce)
                    }
                    #[cfg(windows)]
                    {
                        Err(invalid())
                    }
                };
                let Some(profile) = optional(candidate)? else {
                    return Ok(());
                };
                let cleanup = profile.authorize(None)?;
                cleanup.require_abandoned()?;
                cleanup.delete()
            })();
            if let Err(error) = result {
                diagnostics.push(format!("profile {nonce}: {error}"));
            }
        }
        Ok(diagnostics)
    }
    fn finish_terminal(&self, nonce: &str) -> io::Result<()> {
        let mut file = native::file(
            &self.directory,
            OsStr::new(&terminal_name(nonce)),
            true,
            false,
        )?;
        lock_profile(&file)?;
        let terminal: Terminal = serde_json::from_value(read_json(&mut file)?)?;
        terminal.validate(&self.directory, nonce)?;
        finish(&self.directory, file, terminal, None)
    }
}
fn terminal_name(nonce: &str) -> String {
    format!("{TERMINAL}{nonce}.json")
}
impl RecoveryProfile {
    pub(crate) fn root_identity(&self) -> Id {
        self.root_id
    }
    pub(crate) fn identity(&self) -> Id {
        self.id
    }
    pub(crate) fn open_file(&self, name: &str, write: bool) -> io::Result<File> {
        native::file(&self.directory, OsStr::new(name), write, false)
    }
    pub(crate) fn create_file(&self, name: &str) -> io::Result<File> {
        native::file(&self.directory, OsStr::new(name), true, true)
    }
    pub(crate) fn same_file(&self, name: &str, file: &File) -> bool {
        self.open_file(name, false)
            .and_then(|f| Ok(native::identity(&f)? == native::identity(file)?))
            .unwrap_or(false)
    }
    pub(crate) fn sync(&self) -> io::Result<()> {
        native::sync(&self.directory)
    }
    pub(crate) fn replace_receipt(&self, bytes: &[u8]) -> io::Result<()> {
        let name = format!(".clovy-cancel-{}.tmp", uuid::Uuid::new_v4());
        let mut file = self.create_file(&name)?;
        file.write_all(bytes)?;
        native::sync(&file)?;
        native::rename(
            &self.directory,
            OsStr::new(&name),
            &file,
            &self.directory,
            OsStr::new(RECEIPT),
            true,
        )?;
        self.sync()
    }
    fn validate_links(&self) -> io::Result<()> {
        if native::identity(&self.root)? != self.root_id {
            return Err(invalid());
        }
        match &self.location {
            #[cfg(unix)]
            ProfileLocation::UnixHolder(holder) => {
                native::same(
                    &self.root,
                    OsStr::new(&format!("{HOLDER}{}", self.nonce)),
                    holder,
                )?;
                native::same(holder, OsStr::new("profile"), &self.directory)?;
                validate_holder(holder, &self.nonce, self.root_id, self.id)?;
            }
            _ => {
                native::same(&self.root, OsStr::new(&self.nonce), &self.directory)?;
            }
        }
        Ok(())
    }
    fn authorize(self, held: Option<File>) -> io::Result<Cleanup> {
        let lease = if let Some(file) = held {
            file
        } else {
            let file = self.open_file(LEASE, true)?;
            lock_profile(&file)?;
            file
        };
        self.validate_links()?;
        if !self.same_file(LEASE, &lease) {
            return Err(invalid());
        }
        let mut receipt_file = self.open_file(RECEIPT, false)?;
        let receipt = read_json_bounded(&mut receipt_file, 4096)?;
        validate_receipt(&receipt, &self.nonce, self.root_id, self.id)?;
        let marker = optional(self.open_file(MARKER, false))?;
        match (&marker, receipt["phase"].as_str()) {
            (Some(file), Some("Starting" | "Cancelled" | "Owned")) => {
                let mut bytes = Vec::new();
                file.take(32).read_to_end(&mut bytes)?;
                if bytes != b"Starting\n" {
                    return Err(invalid());
                }
            }
            (None, Some("Reserved")) => (),
            _ => return Err(invalid()),
        }
        let result = Cleanup {
            profile: self,
            lease,
            receipt_file,
            marker,
            receipt,
        };
        result.validate()?;
        checkpoint("authorized");
        Ok(result)
    }
    /// Caller owns a cancelled Slot and has already proved its original process
    /// tree exited. The parent's death is deliberately not a Stop requirement.
    pub(crate) fn delete_stopped(self, held: Option<File>, nonce: &str) -> io::Result<()> {
        if self.nonce != nonce {
            return Err(invalid());
        }
        self.authorize(held)?.delete()
    }
}
fn validate_receipt(value: &Value, nonce: &str, root: Id, profile: Id) -> io::Result<()> {
    let allowed = [
        "version",
        "nonce",
        "ownerProcessId",
        "ownerBirth",
        "phase",
        "rootIdentity",
        "profileIdentity",
        "owned",
    ];
    if !canonical(nonce)
        || !value
            .as_object()
            .is_some_and(|o| o.keys().all(|k| allowed.contains(&k.as_str())))
        || value["version"] != 1
        || value["nonce"] != nonce
        || value["rootIdentity"] != json!(root)
        || value["profileIdentity"] != json!(profile)
        || value["ownerProcessId"]
            .as_u64()
            .and_then(|p| u32::try_from(p).ok())
            .filter(|p| *p > 0 && *p <= i32::MAX as u32)
            .is_none()
        || value["ownerBirth"].as_u64().filter(|b| *b > 0).is_none()
        || !matches!(
            value["phase"].as_str(),
            Some("Reserved" | "Starting" | "Cancelled" | "Owned")
        )
    {
        return Err(invalid());
    }
    if value["phase"] == "Owned" {
        let owned: Owned = serde_json::from_value(value["owned"].clone())?;
        if owned.version != 1
            || owned.nonce != nonce
            || owned.root_identity != root
            || owned.profile_identity != profile
            || value["ownerProcessId"] != owned.parent_pid
            || value["ownerBirth"] != owned.parent_birth
            || owned.host_pid == 0
            || owned.host_pid > i32::MAX as u32
            || owned.host_birth == 0
        {
            return Err(invalid());
        }
        #[cfg(unix)]
        if owned.group != Some(owned.host_pid) || owned.job.is_some() {
            return Err(invalid());
        }
        #[cfg(windows)]
        if owned.group.is_some() || owned.job != Some(format!("Local\\ClovyBrowser-{nonce}")) {
            return Err(invalid());
        }
    } else if value.get("owned").is_some() {
        return Err(invalid());
    }
    Ok(())
}
impl Cleanup {
    fn require_abandoned(&self) -> io::Result<()> {
        if !process_definitely_exited(
            self.receipt["ownerProcessId"]
                .as_u64()
                .ok_or_else(invalid)? as u32,
            self.receipt["ownerBirth"].as_u64().ok_or_else(invalid)?,
        ) {
            return Err(invalid());
        }
        if self.receipt["phase"] == "Owned" {
            let owned: Owned = serde_json::from_value(self.receipt["owned"].clone())?;
            if owned.boot.is_none()
                || owned.boot != boot_identity()?
                || !process_definitely_exited(owned.host_pid, owned.host_birth)
            {
                return Err(invalid());
            }
            #[cfg(unix)]
            if !unix_group_exited(owned.group.ok_or_else(invalid)? as i32) {
                return Err(invalid());
            }
            #[cfg(windows)]
            if !native::job_exited(owned.job.as_deref().ok_or_else(invalid)?)? {
                return Err(invalid());
            }
        }
        self.validate()
    }
    fn validate(&self) -> io::Result<()> {
        self.profile.validate_links()?;
        if !self.profile.same_file(LEASE, &self.lease)
            || !self.profile.same_file(RECEIPT, &self.receipt_file)
            || self
                .marker
                .as_ref()
                .is_some_and(|f| !self.profile.same_file(MARKER, f))
        {
            return Err(invalid());
        }
        Ok(())
    }
    fn cancel_starting(self) -> io::Result<Self> {
        self.validate()?;
        if self.receipt["phase"] != "Starting" {
            return Ok(self);
        }
        // There is deliberately no Cleanup value during publication: deletion
        // requires a captured, revalidated reader, never an optional receipt.
        let Self {
            profile,
            lease,
            receipt_file,
            marker,
            mut receipt,
        } = self;
        receipt["phase"] = json!("Cancelled");
        let bytes = serde_json::to_vec(&receipt)?;
        drop(receipt_file);
        checkpoint("receipt-reader-closed");
        profile.replace_receipt(&bytes)?;
        let mut receipt_file = profile.open_file(RECEIPT, false)?;
        if read_json_bounded(&mut receipt_file, 4096)? != receipt {
            return Err(invalid());
        }
        let cleanup = Self {
            profile,
            lease,
            receipt_file,
            marker,
            receipt,
        };
        cleanup.validate()?;
        checkpoint("cancelled");
        Ok(cleanup)
    }
    fn quarantine(&mut self) -> io::Result<()> {
        self.validate()?;
        #[cfg(windows)]
        if matches!(self.profile.location, ProfileLocation::Original) {
            // NTFS cannot rename the source directory with open descendants.
            // Pin the same physical profile in place, retaining the same lease.
            self.profile.directory = native::pin_directory(&self.profile.directory)?;
            self.profile.location = ProfileLocation::WindowsPinnedOriginal;
            self.validate()?;
            checkpoint("quarantined");
        }
        #[cfg(unix)]
        if matches!(self.profile.location, ProfileLocation::Original) {
            let name = format!("{HOLDER}{}", self.profile.nonce);
            let holder = match native::directory(&self.profile.root, OsStr::new(&name), false) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    // Publish a complete holder without replacing collisions.
                    // A prepublication crash leaves only an unknown temporary
                    // directory, never a blocker in the deterministic namespace.
                    let temporary = format!(".clovy-gc-holder-{}.tmp", uuid::Uuid::new_v4());
                    let holder =
                        native::directory(&self.profile.root, OsStr::new(&temporary), true)?;
                    let receipt = HolderReceipt {
                        version: 1,
                        nonce: self.profile.nonce.clone(),
                        root: self.profile.root_id,
                        holder: native::identity(&holder)?,
                        profile: self.profile.id,
                    };
                    let mut file = native::file(&holder, OsStr::new(HOLDER_RECEIPT), true, true)?;
                    serde_json::to_writer(&mut file, &receipt)?;
                    native::sync(&file)?;
                    drop(file);
                    native::sync(&holder)?;
                    native::rename(
                        &self.profile.root,
                        OsStr::new(&temporary),
                        &holder,
                        &self.profile.root,
                        OsStr::new(&name),
                        false,
                    )?;
                    native::sync(&self.profile.root)?;
                    holder
                }
                Ok(holder) => {
                    validate_holder(
                        &holder,
                        &self.profile.nonce,
                        self.profile.root_id,
                        self.profile.id,
                    )?;
                    if native::list(&holder)?
                        .iter()
                        .any(|e| e.name != OsStr::new(HOLDER_RECEIPT))
                    {
                        return Err(invalid());
                    }
                    holder
                }
                Err(e) => return Err(e),
            };
            self.validate()?;
            native::same(&self.profile.root, OsStr::new(&name), &holder)?;
            checkpoint("holder-created");
            native::rename(
                &self.profile.root,
                OsStr::new(&self.profile.nonce),
                &self.profile.directory,
                &holder,
                OsStr::new("profile"),
                false,
            )?;
            self.profile.location = ProfileLocation::UnixHolder(holder);
            self.validate()?;
            native::sync(&self.profile.root)?;
            if let ProfileLocation::UnixHolder(holder) = &self.profile.location {
                native::sync(holder)?;
            }
            checkpoint("quarantined");
        }
        Ok(())
    }
    fn delete(self) -> io::Result<()> {
        let mut cleanup = self.cancel_starting()?;
        cleanup.quarantine()?;
        cleanup.delete_payload()
    }
    fn delete_payload(self) -> io::Result<()> {
        self.validate()?;
        empty_payload(&self.profile.directory, true)?;
        self.validate()?;
        let mut controls = vec![
            Control {
                name: RECEIPT.to_owned(),
                id: native::identity(&self.receipt_file)?,
            },
            Control {
                name: LEASE.to_owned(),
                id: native::identity(&self.lease)?,
            },
        ];
        if let Some(marker) = &self.marker {
            controls.push(Control {
                name: MARKER.to_owned(),
                id: native::identity(marker)?,
            });
        }
        let terminal = Terminal {
            version: 2,
            phase: "PayloadGone".to_owned(),
            nonce: self.profile.nonce.clone(),
            root: self.profile.root_id,
            location: match &self.profile.location {
                #[cfg(unix)]
                ProfileLocation::UnixHolder(holder) => TerminalLocation::UnixHolder {
                    holder: native::identity(holder)?,
                    holder_receipt: native::identity(&validate_holder(
                        holder,
                        &self.profile.nonce,
                        self.profile.root_id,
                        self.profile.id,
                    )?)?,
                },
                #[cfg(windows)]
                ProfileLocation::WindowsPinnedOriginal => TerminalLocation::WindowsPinnedOriginal,
                ProfileLocation::Original => return Err(invalid()),
            },
            profile: self.profile.id,
            controls,
            receipt: self.receipt.clone(),
        };
        verify_controls(&self.profile.directory, &terminal, false)?;
        checkpoint("payload-gone");
        let temporary = format!(".clovy-gc-publish-{}.tmp", uuid::Uuid::new_v4());
        let mut file = native::file(&self.profile.root, OsStr::new(&temporary), true, true)?;
        lock_profile(&file)?;
        serde_json::to_writer(&mut file, &terminal)?;
        native::sync(&file)?;
        native::rename(
            &self.profile.root,
            OsStr::new(&temporary),
            &file,
            &self.profile.root,
            OsStr::new(&terminal_name(&terminal.nonce)),
            false,
        )?;
        native::sync(&self.profile.root)?;
        checkpoint("terminal-published");
        let root = self.profile.root.try_clone()?;
        finish(&root, file, terminal, Some(self))
    }
}
fn empty_payload(directory: &File, top: bool) -> io::Result<()> {
    for entry in native::list(directory)? {
        if top && CONTROLS.iter().any(|c| entry.name == OsStr::new(c)) {
            continue;
        }
        let file = native::entry(directory, &entry, true)?;
        if entry.directory && !entry.link {
            empty_payload(&file, false)?;
        }
        native::remove(
            directory,
            &entry.name,
            &file,
            entry.directory && !entry.link,
        )?;
        drop(file);
        native::sync(directory)?;
        checkpoint("payload-entry");
    }
    native::sync(directory)
}
impl Terminal {
    fn validate(&self, root: &File, nonce: &str) -> io::Result<()> {
        if self.version != 2
            || self.phase != "PayloadGone"
            || self.nonce != nonce
            || self.root != native::identity(root)?
            || self.profile == [0, 0]
            || self.controls.iter().any(|control| control.id == [0, 0])
            || !matches!(
                self.receipt["phase"].as_str(),
                Some("Reserved" | "Cancelled" | "Owned")
            )
        {
            return Err(invalid());
        }
        match self.location {
            TerminalLocation::UnixHolder {
                holder,
                holder_receipt,
            } if cfg!(unix) && holder != [0, 0] && holder_receipt != [0, 0] => (),
            TerminalLocation::WindowsPinnedOriginal if cfg!(windows) => (),
            _ => return Err(invalid()),
        }
        validate_receipt(&self.receipt, nonce, self.root, self.profile)?;
        let expected = if self.receipt["phase"] == "Reserved" {
            &CONTROLS[..2]
        } else {
            &CONTROLS[..]
        };
        if self.controls.len() != expected.len()
            || expected
                .iter()
                .any(|name| self.controls.iter().filter(|c| c.name == *name).count() != 1)
        {
            return Err(invalid());
        }
        Ok(())
    }
}
fn verify_controls(directory: &File, terminal: &Terminal, missing: bool) -> io::Result<()> {
    let entries = native::list(directory)?;
    if !missing && entries.len() != terminal.controls.len() {
        return Err(invalid());
    }
    for entry in entries {
        if entry.directory
            || entry.link
            || !terminal
                .controls
                .iter()
                .any(|c| entry.name == OsStr::new(&c.name) && entry.id == c.id)
        {
            return Err(invalid());
        }
        let file = native::entry(directory, &entry, false)?;
        if entry.name == OsStr::new(RECEIPT)
            && read_json_bounded(&mut file.try_clone()?, 4096)? != terminal.receipt
        {
            return Err(invalid());
        }
        if entry.name == OsStr::new(MARKER) {
            let mut bytes = Vec::new();
            (&file).take(32).read_to_end(&mut bytes)?;
            if bytes != b"Starting\n" {
                return Err(invalid());
            }
        }
    }
    Ok(())
}
fn validate_holder(holder: &File, nonce: &str, root: Id, profile: Id) -> io::Result<File> {
    let mut file = native::file(holder, OsStr::new(HOLDER_RECEIPT), false, false)?;
    let receipt: HolderReceipt = serde_json::from_value(read_json(&mut file)?)?;
    if receipt
        != (HolderReceipt {
            version: 1,
            nonce: nonce.to_owned(),
            root,
            holder: native::identity(holder)?,
            profile,
        })
    {
        return Err(invalid());
    }
    native::same(holder, OsStr::new(HOLDER_RECEIPT), &file)?;
    Ok(file)
}
fn finish(root: &File, record: File, terminal: Terminal, held: Option<Cleanup>) -> io::Result<()> {
    terminal.validate(root, &terminal.nonce)?;
    native::same(root, OsStr::new(&terminal_name(&terminal.nonce)), &record)?;
    match terminal.location {
        TerminalLocation::WindowsPinnedOriginal => {
            finish_profile(root, OsStr::new(&terminal.nonce), &terminal, held)?;
            if optional(native::directory(root, OsStr::new(&terminal.nonce), false))?.is_some() {
                return Err(invalid());
            }
        }
        TerminalLocation::UnixHolder {
            holder,
            holder_receipt,
        } => {
            finish_holder(root, &terminal, holder, holder_receipt, held)?;
        }
    }
    native::sync(root)?;
    native::remove(
        root,
        OsStr::new(&terminal_name(&terminal.nonce)),
        &record,
        false,
    )?;
    drop(record);
    native::sync(root)
}
fn finish_holder(
    root: &File,
    terminal: &Terminal,
    holder_id: Id,
    holder_receipt: Id,
    held: Option<Cleanup>,
) -> io::Result<()> {
    let holder_name = format!("{HOLDER}{}", terminal.nonce);
    if let Some(holder) = optional(native::directory(root, OsStr::new(&holder_name), false))? {
        if native::identity(&holder)? != holder_id {
            return Err(invalid());
        }
        let entries = native::list(&holder)?;
        if entries.iter().any(|e| {
            if e.name == OsStr::new("profile") {
                !e.directory || e.link || e.id != terminal.profile
            } else {
                e.name != OsStr::new(HOLDER_RECEIPT)
                    || e.directory
                    || e.link
                    || e.id != holder_receipt
            }
        }) {
            return Err(invalid());
        }
        if entries.iter().any(|e| e.name == OsStr::new("profile")) {
            let marker =
                validate_holder(&holder, &terminal.nonce, terminal.root, terminal.profile)?;
            if native::identity(&marker)? != holder_receipt {
                return Err(invalid());
            }
        }
        finish_profile(&holder, OsStr::new("profile"), terminal, held)?;
        if let Some(file) = optional(native::file(
            &holder,
            OsStr::new(HOLDER_RECEIPT),
            true,
            false,
        ))? {
            if native::identity(&file)? != holder_receipt {
                return Err(invalid());
            }
            validate_holder(&holder, &terminal.nonce, terminal.root, terminal.profile)?;
            native::remove(&holder, OsStr::new(HOLDER_RECEIPT), &file, false)?;
            drop(file);
            native::sync(&holder)?;
            checkpoint("holder-control-removed");
        }
        if !native::list(&holder)?.is_empty() {
            return Err(invalid());
        }
        native::same(root, OsStr::new(&holder_name), &holder)?;
        native::remove_directory(root, OsStr::new(&holder_name), &holder)?;
        drop(holder);
        native::sync(root)?;
        checkpoint("holder-removed");
    }
    if optional(native::directory(root, OsStr::new(&holder_name), false))?.is_some() {
        return Err(invalid());
    }
    Ok(())
}
fn finish_profile(
    parent: &File,
    name: &OsStr,
    terminal: &Terminal,
    mut held: Option<Cleanup>,
) -> io::Result<()> {
    let Some(directory) = optional(native::directory(parent, name, false))? else {
        // Also make the absence durable when resuming after directory removal.
        return native::sync(parent);
    };
    if native::identity(&directory)? != terminal.profile {
        return Err(invalid());
    }
    #[cfg(windows)]
    let directory = native::pin_directory(&directory)?;
    verify_controls(&directory, terminal, true)?;
    // A missing lease is never recreated. The locked terminal owns exclusion.
    let lease = if held.is_none() {
        let lease = optional(native::file(&directory, OsStr::new(LEASE), true, false))?;
        if let Some(file) = &lease {
            lock_profile(file)?;
        }
        lease
    } else {
        None
    };
    verify_controls(&directory, terminal, true)?;
    for control in &terminal.controls {
        if let Some(file) = optional(native::file(
            &directory,
            OsStr::new(&control.name),
            true,
            false,
        ))? {
            if native::identity(&file)? != control.id {
                return Err(invalid());
            }
            native::remove(&directory, OsStr::new(&control.name), &file, false)?;
            drop(file);
            native::sync(&directory)?;
            checkpoint(&format!("removed-{}", control.name));
        }
    }
    drop(lease);
    drop(held.take());
    native::sync(&directory)?;
    checkpoint("profile-retiring");
    // Only the durable, exclusively locked terminal permits releasing this
    // pin. Keep a reference to the same object across release; no path reopen.
    #[cfg(windows)]
    let directory = native::unpin_directory(directory)?;
    #[cfg(windows)]
    checkpoint("pin-closed");
    native::same(parent, name, &directory)?;
    if !native::list(&directory)?.is_empty() {
        return Err(invalid());
    }
    native::remove_directory(parent, name, &directory)?;
    drop(directory);
    native::sync(parent)?;
    checkpoint("profile-removed");
    Ok(())
}

// Deterministic crash/swap barriers exist only in test binaries. Production has
// no environment-controlled fault injection or recovery override.
pub(crate) fn checkpoint(_name: &str) {
    #[cfg(test)]
    tests::checkpoint(_name);
}

#[cfg(all(test, unix))]
mod tests {
    use super::super::clovy_browser_process_identity::process_birth;
    use super::*;
    use std::{
        cell::RefCell,
        io::{BufRead, BufReader},
        os::unix::{fs::symlink, process::CommandExt},
        process::{Command, Stdio},
    };
    fn physical_tempdir() -> tempfile::TempDir {
        // Resolve platform temp aliases before creating fixtures or adversarial links.
        let base = std::env::temp_dir().canonicalize().unwrap();
        tempfile::tempdir_in(base).unwrap()
    }
    type Hook = Box<dyn FnMut(&str)>;
    thread_local! { static HOOK: RefCell<Option<Hook>> = RefCell::new(None); }
    pub(super) fn checkpoint(name: &str) {
        HOOK.with(|hook| {
            if let Some(f) = hook.borrow_mut().as_mut() {
                f(name);
            }
        });
        if std::env::var("CLOVY_GC_CHILD").is_ok()
            && std::env::var("CLOVY_GC_CRASH_AT").ok().as_deref() == Some(name)
        {
            std::process::exit(77);
        }
        if std::env::var("CLOVY_GC_CHILD").is_ok()
            && std::env::var("CLOVY_GC_PAUSE_AT").ok().as_deref() == Some(name)
        {
            println!("GC_BARRIER");
            std::io::stdout().flush().unwrap();
            std::io::stdin().read_exact(&mut [0]).unwrap();
        }
    }
    fn dead() -> (u32, u64) {
        let mut child = Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let value = (child.id(), process_birth(child.id()).unwrap());
        child.kill().unwrap();
        child.wait().unwrap();
        value
    }
    fn fixture(root: &Path, phase: &str, alive: bool) -> (String, RecoveryProfile) {
        let root = RecoveryRoot::open(root).unwrap();
        let nonce = uuid::Uuid::new_v4().to_string();
        let profile = root.create_profile(OsStr::new(&nonce)).unwrap();
        profile.create_file(LEASE).unwrap().sync_all().unwrap();
        let (pid, birth) = if alive {
            (
                std::process::id(),
                process_birth(std::process::id()).unwrap(),
            )
        } else {
            dead()
        };
        let mut receipt = json!({"version":1,"nonce":nonce,"ownerProcessId":pid,"ownerBirth":birth,"phase":phase,"rootIdentity":profile.root_id,"profileIdentity":profile.id});
        if phase == "Owned" {
            let (host, host_birth) = dead();
            receipt["owned"] = json!(Owned {
                version: 1,
                nonce: nonce.clone(),
                root_identity: profile.root_id,
                profile_identity: profile.id,
                parent_pid: pid,
                parent_birth: birth,
                host_pid: host,
                host_birth,
                group: Some(host),
                job: None,
                boot: boot_identity().unwrap()
            });
        }
        let mut file = profile.create_file(RECEIPT).unwrap();
        serde_json::to_writer(&mut file, &receipt).unwrap();
        file.sync_all().unwrap();
        if phase != "Reserved" {
            profile
                .create_file(MARKER)
                .unwrap()
                .write_all(b"Starting\n")
                .unwrap();
        }
        profile.sync().unwrap();
        (nonce, profile)
    }
    fn payload(root: &Path, nonce: &str) {
        let dir = root.join(nonce);
        std::fs::create_dir(dir.join("nested")).unwrap();
        for name in CONTROLS {
            std::fs::write(dir.join("nested").join(name), b"payload, not a control").unwrap();
        }
        std::fs::write(dir.join("payload"), b"synthetic").unwrap();
    }
    fn child(root: &Path, crash: Option<&str>, pause: Option<&str>) -> std::process::Child {
        let test = format!(
            "{}::process_fixture",
            module_path!().split_once("::").unwrap().1
        );
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.args(["--exact", &test, "--nocapture"])
            .env("CLOVY_GC_CHILD", root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped());
        if let Some(name) = crash {
            cmd.env("CLOVY_GC_CRASH_AT", name);
        }
        if let Some(name) = pause {
            cmd.env("CLOVY_GC_PAUSE_AT", name);
        }
        cmd.spawn().unwrap()
    }
    #[test]
    fn process_fixture() {
        let Some(path) = std::env::var_os("CLOVY_GC_CHILD") else {
            return;
        };
        if std::env::var("CLOVY_GC_ROLE").ok().as_deref() == Some("owner") {
            let (nonce, _profile) = fixture(Path::new(&path), "Starting", true);
            let grant = super::super::clovy_browser_grant::Grant::new(
                &Path::new(&path).join(&nonce),
                false,
            )
            .unwrap();
            println!("GRANT={}", serde_json::to_string(&grant).unwrap());
            std::io::stdout().flush().unwrap();
            std::io::stdin().read_exact(&mut [0]).unwrap();
            std::process::exit(0);
        }
        if std::env::var("CLOVY_GC_ROLE").ok().as_deref() == Some("host") {
            let mut line = String::new();
            std::io::stdin().read_line(&mut line).unwrap();
            let grant: super::super::clovy_browser_grant::Grant =
                serde_json::from_str(&line).unwrap();
            if std::env::var_os("CLOVY_GC_SWAP_PROFILE").is_some() {
                let profile = grant.profile.clone();
                HOOK.with(|hook| {
                    *hook.borrow_mut() = Some(Box::new(move |at| {
                        if at == "grant-before-owned" {
                            std::fs::rename(&profile, profile.with_file_name("captured-original"))
                                .unwrap();
                            std::fs::create_dir(&profile).unwrap();
                            std::fs::write(profile.join("sentinel"), b"replacement").unwrap();
                        }
                    }))
                });
            }
            assert!(
                grant.acquire_owned(None).is_err(),
                "cancelled/dead-owner host was admitted"
            );
            return;
        }
        let root = RecoveryRoot::open(Path::new(&path)).unwrap();
        let diagnostics = root.recover().unwrap();
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }
    fn barrier(output: &mut impl BufRead, prefix: &str) -> String {
        loop {
            let mut line = String::new();
            assert!(
                output.read_line(&mut line).unwrap() > 0,
                "process exited before {prefix}"
            );
            if let Some((_, value)) = line.split_once(prefix) {
                return value.trim().to_owned();
            }
        }
    }
    #[test]
    fn real_bootstrap_before_lock_is_cancelled_and_after_lock_excludes_gc() {
        let test = format!(
            "{}::process_fixture",
            module_path!().split_once("::").unwrap().1
        );
        for boundary in ["grant-before-lock", "grant-after-lock"] {
            let temp = physical_tempdir();
            let mut owner = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", &test, "--nocapture"])
                .env("CLOVY_GC_CHILD", temp.path())
                .env("CLOVY_GC_ROLE", "owner")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let grant = barrier(&mut BufReader::new(owner.stdout.take().unwrap()), "GRANT=");
            let nonce = serde_json::from_str::<Value>(&grant).unwrap()["nonce"]
                .as_str()
                .unwrap()
                .to_owned();
            let mut host = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", &test, "--nocapture"])
                .env("CLOVY_GC_CHILD", temp.path())
                .env("CLOVY_GC_ROLE", "host")
                .env("CLOVY_GC_PAUSE_AT", boundary)
                .process_group(0)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            writeln!(host.stdin.as_mut().unwrap(), "{grant}").unwrap();
            let mut host_output = BufReader::new(host.stdout.take().unwrap());
            barrier(&mut host_output, "GC_BARRIER");
            owner.stdin.take().unwrap().write_all(b"x").unwrap();
            assert!(owner.wait().unwrap().success());
            let root = RecoveryRoot::open(temp.path()).unwrap();
            let d = root.recover().unwrap();
            if boundary == "grant-before-lock" {
                assert!(d.is_empty());
                assert!(!temp.path().join(&nonce).exists());
            } else {
                assert!(!d.is_empty());
                assert!(temp.path().join(&nonce).is_dir());
            }
            host.stdin.take().unwrap().write_all(b"x").unwrap();
            assert!(host.wait().unwrap().success());
            assert!(root.recover().unwrap().is_empty());
            assert!(std::fs::read_dir(temp.path()).unwrap().next().is_none());
        }
    }
    #[test]
    fn host_owned_publication_uses_captured_profile_after_name_swap() {
        let test = format!(
            "{}::process_fixture",
            module_path!().split_once("::").unwrap().1
        );
        let temp = physical_tempdir();
        let mut owner = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &test, "--nocapture"])
            .env("CLOVY_GC_CHILD", temp.path())
            .env("CLOVY_GC_ROLE", "owner")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut owner_output = BufReader::new(owner.stdout.take().unwrap());
        let grant = barrier(&mut owner_output, "GRANT=");
        let nonce = serde_json::from_str::<Value>(&grant).unwrap()["nonce"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut host = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &test, "--nocapture"])
            .env("CLOVY_GC_CHILD", temp.path())
            .env("CLOVY_GC_ROLE", "host")
            .env("CLOVY_GC_SWAP_PROFILE", "1")
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        writeln!(host.stdin.take().unwrap(), "{grant}").unwrap();
        let output = host.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let replacement = temp.path().join(nonce);
        assert_eq!(
            std::fs::read(replacement.join("sentinel")).unwrap(),
            b"replacement"
        );
        assert!(!replacement.join(RECEIPT).exists());
        assert!(!replacement.join(LEASE).exists());
        let original: Value = serde_json::from_slice(
            &std::fs::read(temp.path().join("captured-original").join(RECEIPT)).unwrap(),
        )
        .unwrap();
        assert_eq!(original["phase"], "Owned");
        owner.stdin.take().unwrap().write_all(b"x").unwrap();
        assert!(owner.wait().unwrap().success());
    }
    #[test]
    fn crash_after_durable_owned_before_admission_returns_is_recoverable() {
        let test = format!(
            "{}::process_fixture",
            module_path!().split_once("::").unwrap().1
        );
        let temp = physical_tempdir();
        let mut owner = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &test, "--nocapture"])
            .env("CLOVY_GC_CHILD", temp.path())
            .env("CLOVY_GC_ROLE", "owner")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut owner_output = BufReader::new(owner.stdout.take().unwrap());
        let grant = barrier(&mut owner_output, "GRANT=");
        let nonce = serde_json::from_str::<Value>(&grant).unwrap()["nonce"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut host = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &test, "--nocapture"])
            .env("CLOVY_GC_CHILD", temp.path())
            .env("CLOVY_GC_ROLE", "host")
            .env("CLOVY_GC_CRASH_AT", "grant-owned-published")
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        writeln!(host.stdin.take().unwrap(), "{grant}").unwrap();
        let output = host.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(77));
        let receipt: Value =
            serde_json::from_slice(&std::fs::read(temp.path().join(&nonce).join(RECEIPT)).unwrap())
                .unwrap();
        assert_eq!(receipt["phase"], "Owned");
        let root = RecoveryRoot::open(temp.path()).unwrap();
        assert!(!root.recover().unwrap().is_empty());
        assert!(temp.path().join(&nonce).exists());
        owner.stdin.take().unwrap().write_all(b"x").unwrap();
        assert!(owner.wait().unwrap().success());
        assert!(root.recover().unwrap().is_empty());
        assert!(!temp.path().join(nonce).exists());
    }
    #[test]
    fn crash_each_durable_boundary_resumes_in_another_process() {
        for phase in ["Reserved", "Starting", "Owned", "Cancelled"] {
            let boundaries = [
                "authorized",
                "holder-created",
                "quarantined",
                "payload-entry",
                "payload-gone",
                "terminal-published",
                "removed-.clovy-reserved.json",
                "removed-.clovy-lease",
                "profile-removed",
                "holder-control-removed",
                "holder-removed",
            ];
            for boundary in boundaries
                .into_iter()
                .chain((phase == "Starting").then_some("cancelled"))
                .chain((phase == "Starting").then_some("receipt-reader-closed"))
                .chain(Some("profile-retiring"))
                .chain((phase != "Reserved").then_some("removed-.clovy-starting"))
            {
                let temp = physical_tempdir();
                let (nonce, _) = fixture(temp.path(), phase, false);
                payload(temp.path(), &nonce);
                let output = child(temp.path(), Some(boundary), None)
                    .wait_with_output()
                    .unwrap();
                assert_eq!(
                    output.status.code(),
                    Some(77),
                    "{phase} {boundary}: {}",
                    String::from_utf8_lossy(&output.stdout)
                );
                let output = child(temp.path(), None, None).wait_with_output().unwrap();
                assert!(
                    output.status.success(),
                    "{phase} {boundary}: {}",
                    String::from_utf8_lossy(&output.stdout)
                );
                assert!(
                    std::fs::read_dir(temp.path()).unwrap().next().is_none(),
                    "{phase} {boundary}"
                );
            }
        }
    }
    #[test]
    fn stop_does_not_require_local_parent_death_and_never_follows_payload_links() {
        let temp = physical_tempdir();
        let outside = physical_tempdir();
        std::fs::write(outside.path().join("sentinel"), b"keep").unwrap();
        let (nonce, profile) = fixture(temp.path(), "Starting", true);
        payload(temp.path(), &nonce);
        symlink(outside.path(), temp.path().join(&nonce).join("link")).unwrap();
        profile.delete_stopped(None, &nonce).unwrap();
        assert!(!temp.path().join(nonce).exists());
        assert_eq!(
            std::fs::read(outside.path().join("sentinel")).unwrap(),
            b"keep"
        );
    }
    #[test]
    fn terminal_payload_extra_is_preserved_and_lease_is_never_recreated() {
        for boundary in [
            "terminal-published",
            "removed-.clovy-lease",
            "profile-removed",
            "holder-removed",
        ] {
            let temp = physical_tempdir();
            let (nonce, _) = fixture(temp.path(), "Starting", false);
            assert_eq!(
                child(temp.path(), Some(boundary), None)
                    .wait()
                    .unwrap()
                    .code(),
                Some(77)
            );
            let profile = temp.path().join(format!("{HOLDER}{nonce}/profile"));
            if profile.is_dir() {
                let missing = !profile.join(LEASE).exists();
                std::fs::write(profile.join("unexpected"), b"keep").unwrap();
                let d = RecoveryRoot::open(temp.path()).unwrap().recover().unwrap();
                assert!(!d.is_empty());
                assert_eq!(std::fs::read(profile.join("unexpected")).unwrap(), b"keep");
                if missing {
                    assert!(!profile.join(LEASE).exists());
                }
            } else {
                assert!(RecoveryRoot::open(temp.path())
                    .unwrap()
                    .recover()
                    .unwrap()
                    .is_empty());
            }
        }
    }
    #[test]
    fn concurrent_collectors_exclude_before_and_after_lease_removal() {
        for boundary in ["authorized", "terminal-published", "removed-.clovy-lease"] {
            let temp = physical_tempdir();
            let (nonce, _) = fixture(temp.path(), "Starting", false);
            let mut first = child(temp.path(), None, Some(boundary));
            let mut output = BufReader::new(first.stdout.take().unwrap());
            loop {
                let mut line = String::new();
                assert!(output.read_line(&mut line).unwrap() > 0);
                if line.contains("GC_BARRIER") {
                    break;
                }
            }
            let diagnostics = RecoveryRoot::open(temp.path()).unwrap().recover().unwrap();
            assert!(!diagnostics.is_empty());
            assert!(
                temp.path().join(&nonce).is_dir()
                    || temp
                        .path()
                        .join(format!("{HOLDER}{nonce}/profile"))
                        .is_dir()
            );
            first.stdin.take().unwrap().write_all(b"x").unwrap();
            assert!(first.wait().unwrap().success());
            assert!(std::fs::read_dir(temp.path()).unwrap().next().is_none());
        }
    }
    #[test]
    fn terminal_legacy_or_wrong_platform_location_never_falls_back_to_payload_gc() {
        for change in ["legacy", "location"] {
            let temp = physical_tempdir();
            let (nonce, _) = fixture(temp.path(), "Starting", false);
            assert_eq!(
                child(temp.path(), Some("terminal-published"), None)
                    .wait()
                    .unwrap()
                    .code(),
                Some(77)
            );
            let record = temp.path().join(terminal_name(&nonce));
            let mut value: Value =
                serde_json::from_slice(&std::fs::read(&record).unwrap()).unwrap();
            if change == "legacy" {
                value["version"] = json!(1);
            } else {
                value["location"] = json!({"kind":"windowsPinnedOriginal"});
            }
            std::fs::write(&record, serde_json::to_vec(&value).unwrap()).unwrap();
            let profile = temp.path().join(format!("{HOLDER}{nonce}/profile"));
            std::fs::write(profile.join("unexpected"), b"keep").unwrap();
            assert!(!RecoveryRoot::open(temp.path())
                .unwrap()
                .recover()
                .unwrap()
                .is_empty());
            assert_eq!(std::fs::read(profile.join("unexpected")).unwrap(), b"keep");
            assert!(profile.join(LEASE).exists());
            assert!(record.exists());
        }
    }
    #[test]
    fn terminal_rejects_replaced_controls_and_holder_without_deleting_them() {
        for object in [RECEIPT, LEASE, MARKER, "profile", "holder"] {
            let temp = physical_tempdir();
            let (nonce, _) = fixture(temp.path(), "Starting", false);
            assert_eq!(
                child(temp.path(), Some("terminal-published"), None)
                    .wait()
                    .unwrap()
                    .code(),
                Some(77)
            );
            let holder = temp.path().join(format!("{HOLDER}{nonce}"));
            let profile = holder.join("profile");
            let target = match object {
                "holder" => holder.clone(),
                "profile" => profile.clone(),
                name => profile.join(name),
            };
            let saved = temp.path().join("saved-original");
            std::fs::rename(&target, &saved).unwrap();
            if object == "holder" || object == "profile" {
                std::fs::create_dir(&target).unwrap();
                std::fs::write(target.join("sentinel"), b"keep").unwrap();
            } else {
                std::fs::write(&target, std::fs::read(&saved).unwrap()).unwrap();
            }
            assert!(!RecoveryRoot::open(temp.path())
                .unwrap()
                .recover()
                .unwrap()
                .is_empty());
            assert!(target.exists());
            assert!(saved.exists());
            assert!(temp.path().join(terminal_name(&nonce)).exists());
        }
    }
    #[test]
    fn rename_collision_and_invalid_components_preserve_unknown_entries() {
        let temp = physical_tempdir();
        let root = RecoveryRoot::open(temp.path()).unwrap();
        for name in ["", ".", "..", "a/b", "not-a-uuid"] {
            assert!(root.profile(OsStr::new(name)).is_err());
            assert!(root.create_profile(OsStr::new(name)).is_err());
        }
        let (nonce, profile) = fixture(temp.path(), "Starting", true);
        let holder = temp.path().join(format!("{HOLDER}{nonce}"));
        let collision = holder.join("profile");
        let target = collision.clone();
        HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |at| {
                if at == "holder-created" {
                    std::fs::create_dir(&target).unwrap();
                    std::fs::write(target.join("sentinel"), b"keep").unwrap();
                }
            }))
        });
        let result = profile.delete_stopped(None, &nonce);
        HOOK.with(|hook| *hook.borrow_mut() = None);
        assert!(result.is_err());
        assert!(temp.path().join(nonce).is_dir());
        assert_eq!(std::fs::read(collision.join("sentinel")).unwrap(), b"keep");
    }
    #[test]
    fn substitutions_and_unknown_holder_never_authorize_replacement() {
        for object in ["profile", "holder", LEASE, RECEIPT, MARKER] {
            let temp = physical_tempdir();
            let outside = physical_tempdir();
            std::fs::write(outside.path().join("sentinel"), b"external").unwrap();
            let (nonce, profile) = fixture(temp.path(), "Starting", true);
            payload(temp.path(), &nonce);
            let original = temp.path().join(&nonce);
            let root = temp.path().to_owned();
            let external = outside.path().to_owned();
            let object = object.to_owned();
            let hook_object = object.clone();
            let hook_original = original.clone();
            HOOK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move |at| {
                    if at
                        != if hook_object == "holder" {
                            "quarantined"
                        } else {
                            "authorized"
                        }
                    {
                        return;
                    }
                    let target = match hook_object.as_str() {
                        "profile" => hook_original.clone(),
                        "holder" => root.join(format!("{HOLDER}{nonce}")),
                        name => hook_original.join(name),
                    };
                    let saved = target.with_extension("saved");
                    std::fs::rename(&target, saved).unwrap();
                    if hook_object == "profile" || hook_object == "holder" {
                        symlink(&external, &target).unwrap();
                    } else {
                        symlink(external.join("sentinel"), &target).unwrap();
                    }
                }))
            });
            let result = profile.delete_stopped(None, original_nonce(&original));
            HOOK.with(|hook| *hook.borrow_mut() = None);
            assert!(result.is_err(), "{object}");
            assert_eq!(
                std::fs::read(outside.path().join("sentinel")).unwrap(),
                b"external"
            );
        }
        let temp = physical_tempdir();
        let (nonce, profile) = fixture(temp.path(), "Reserved", true);
        let holder = temp.path().join(format!("{HOLDER}{nonce}"));
        std::fs::create_dir(&holder).unwrap();
        std::fs::write(holder.join("sentinel"), b"unknown").unwrap();
        assert!(profile.delete_stopped(None, &nonce).is_err());
        assert!(holder.join("sentinel").exists());
    }
    fn original_nonce(path: &Path) -> &str {
        path.file_name().unwrap().to_str().unwrap()
    }
    #[test]
    fn captured_ancestor_and_root_swaps_only_delete_original_profile() {
        for ancestor in [false, true] {
            let temp = physical_tempdir();
            let external = physical_tempdir();
            let root = temp.path().join("ancestor/root");
            std::fs::create_dir_all(&root).unwrap();
            let (nonce, profile) = fixture(&root, "Starting", true);
            payload(&root, &nonce);
            let swap = if ancestor {
                root.parent().unwrap().to_owned()
            } else {
                root.clone()
            };
            let saved = temp.path().join("captured");
            std::fs::rename(&swap, &saved).unwrap();
            symlink(external.path(), &swap).unwrap();
            std::fs::write(external.path().join("sentinel"), b"keep").unwrap();
            profile.delete_stopped(None, &nonce).unwrap();
            assert_eq!(
                std::fs::read(external.path().join("sentinel")).unwrap(),
                b"keep"
            );
            assert!(!saved
                .join(if ancestor {
                    format!("root/{nonce}")
                } else {
                    nonce
                })
                .exists());
        }
    }
    #[test]
    fn owned_free_lease_with_live_descendant_is_preserved_until_group_exits() {
        let temp = physical_tempdir();
        let (nonce, profile) = fixture(temp.path(), "Owned", false);
        let mut host = Command::new("/bin/sh")
            .args(["-c", "sleep 60 & echo $!; read x"])
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let host_birth = process_birth(host.id()).unwrap();
        let mut line = String::new();
        BufReader::new(host.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let descendant: u32 = line.trim().parse().unwrap();
        let mut receipt = read_json(&mut profile.open_file(RECEIPT, false).unwrap()).unwrap();
        receipt["owned"]["hostPid"] = json!(host.id());
        receipt["owned"]["hostBirth"] = json!(host_birth);
        receipt["owned"]["group"] = json!(host.id());
        profile
            .replace_receipt(&serde_json::to_vec(&receipt).unwrap())
            .unwrap();
        host.kill().unwrap();
        host.wait().unwrap();
        let root = RecoveryRoot::open(temp.path()).unwrap();
        assert!(!root.recover().unwrap().is_empty());
        assert!(temp.path().join(&nonce).is_dir());
        assert!(process_birth(descendant).is_ok());
        #[cfg(target_os = "macos")]
        let (exit_queue, mut exit_event) = {
            use std::os::fd::{AsRawFd, FromRawFd};
            let fd = unsafe { libc::kqueue() };
            assert!(fd >= 0);
            let queue = unsafe { File::from_raw_fd(fd) };
            let event = libc::kevent {
                ident: descendant as usize,
                filter: libc::EVFILT_PROC,
                flags: libc::EV_ADD | libc::EV_ENABLE | libc::EV_ONESHOT,
                fflags: libc::NOTE_EXIT,
                data: 0,
                udata: std::ptr::null_mut(),
            };
            // Register while alive so even an immediate exit is observed.
            assert_eq!(
                unsafe {
                    libc::kevent(
                        queue.as_raw_fd(),
                        &event,
                        1,
                        std::ptr::null_mut(),
                        0,
                        std::ptr::null(),
                    )
                },
                0
            );
            (queue, event)
        };
        assert_eq!(unsafe { libc::kill(descendant as i32, libc::SIGKILL) }, 0);
        #[cfg(target_os = "macos")]
        {
            use std::os::fd::AsRawFd;
            let timeout = libc::timespec {
                tv_sec: 5,
                tv_nsec: 0,
            };
            assert_eq!(
                unsafe {
                    libc::kevent(
                        exit_queue.as_raw_fd(),
                        std::ptr::null(),
                        0,
                        &mut exit_event,
                        1,
                        &timeout,
                    )
                },
                1
            );
            // Copy fields before assertions because Darwin's kevent may be packed.
            let (ident, filter, flags, fflags) = (
                exit_event.ident,
                exit_event.filter,
                exit_event.flags,
                exit_event.fflags,
            );
            assert_eq!(flags & libc::EV_ERROR, 0);
            assert_eq!(ident, descendant as usize);
            assert_eq!(filter, libc::EVFILT_PROC);
            assert_ne!(fflags & libc::NOTE_EXIT, 0);
        }
        // pidfd observes precisely this descendant, including its unreaped exit.
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::{AsRawFd, FromRawFd};
            let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, descendant, 0) } as i32;
            assert!(fd >= 0);
            let fd = unsafe { File::from_raw_fd(fd) };
            let mut poll = libc::pollfd {
                fd: fd.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            assert_eq!(unsafe { libc::poll(&mut poll, 1, 5000) }, 1);
        }
        assert!(root.recover().unwrap().is_empty());
        assert!(!temp.path().join(nonce).exists());
    }
    #[test]
    fn alive_busy_birth_mismatch_boot_corrupt_legacy_links_are_preserved() {
        let temp = physical_tempdir();
        let root = RecoveryRoot::open(temp.path()).unwrap();
        let mut keep = Vec::new();
        for phase in ["Reserved", "Starting", "Cancelled", "Owned"] {
            keep.push(fixture(temp.path(), phase, true).0);
        }
        let (busy, p) = fixture(temp.path(), "Starting", false);
        let lease = p.open_file(LEASE, true).unwrap();
        lock_profile(&lease).unwrap();
        keep.push(busy);
        let (mismatch, p) = fixture(temp.path(), "Owned", false);
        let mut r = read_json(&mut p.open_file(RECEIPT, false).unwrap()).unwrap();
        r["ownerProcessId"] = json!(std::process::id());
        r["ownerBirth"] = json!(1);
        r["owned"]["parentPid"] = r["ownerProcessId"].clone();
        r["owned"]["parentBirth"] = json!(1);
        p.replace_receipt(&serde_json::to_vec(&r).unwrap()).unwrap();
        keep.push(mismatch);
        let (boot, p) = fixture(temp.path(), "Owned", false);
        let mut r = read_json(&mut p.open_file(RECEIPT, false).unwrap()).unwrap();
        r["owned"]["boot"] = json!("ambiguous");
        p.replace_receipt(&serde_json::to_vec(&r).unwrap()).unwrap();
        keep.push(boot);
        let (corrupt, p) = fixture(temp.path(), "Reserved", false);
        p.replace_receipt(b"not json").unwrap();
        keep.push(corrupt);
        let legacy = uuid::Uuid::new_v4().to_string();
        std::fs::create_dir(temp.path().join(&legacy)).unwrap();
        keep.push(legacy);
        let external = physical_tempdir();
        std::fs::write(external.path().join("sentinel"), b"keep").unwrap();
        let link = uuid::Uuid::new_v4().to_string();
        symlink(external.path(), temp.path().join(&link)).unwrap();
        keep.push(link);
        assert!(!root.recover().unwrap().is_empty());
        for name in keep {
            assert!(temp.path().join(name).exists());
        }
        assert!(external.path().join("sentinel").exists());
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::super::clovy_browser_process_identity::process_birth;
    use super::*;
    use std::{
        cell::RefCell,
        io::{BufRead, BufReader},
        process::{Child, Command, Stdio},
    };
    type Hook = Box<dyn FnMut(&str)>;
    thread_local! { static HOOK: RefCell<Option<Hook>> = RefCell::new(None); }
    pub(super) fn checkpoint(name: &str) {
        HOOK.with(|hook| {
            if let Some(hook) = hook.borrow_mut().as_mut() {
                hook(name);
            }
        });
        if std::env::var_os("CLOVY_WIN_GC_CHILD").is_none() {
            return;
        }
        if std::env::var("CLOVY_WIN_GC_CRASH").ok().as_deref() == Some(name) {
            std::process::exit(77);
        }
        if std::env::var("CLOVY_WIN_GC_PAUSE").ok().as_deref() == Some(name) {
            println!("GC_BARRIER");
            std::io::stdout().flush().unwrap();
            std::io::stdin().read_exact(&mut [0]).unwrap();
        }
    }
    fn command(path: &Path, role: &str) -> Command {
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.args([
            "--exact",
            &format!(
                "{}::process_fixture",
                module_path!().split_once("::").unwrap().1
            ),
            "--nocapture",
        ])
        .env("CLOVY_WIN_GC_CHILD", path)
        .env("CLOVY_WIN_GC_ROLE", role)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
        cmd
    }
    fn barrier(output: &mut impl BufRead, prefix: &str) -> String {
        loop {
            let mut line = String::new();
            assert!(
                output.read_line(&mut line).unwrap() > 0,
                "child exited before {prefix}"
            );
            if let Some((_, value)) = line.split_once(prefix) {
                return value.trim().to_owned();
            }
        }
    }
    fn dead(path: &Path) -> (u32, u64) {
        let mut child = command(path, "wait").spawn().unwrap();
        let birth = barrier(&mut BufReader::new(child.stdout.take().unwrap()), "BIRTH=")
            .parse()
            .unwrap();
        let pid = child.id();
        child.kill().unwrap();
        child.wait().unwrap();
        (pid, birth)
    }
    fn fixture(path: &Path, phase: &str, alive: bool) -> (String, RecoveryProfile) {
        let root = RecoveryRoot::open(path).unwrap();
        let nonce = uuid::Uuid::new_v4().to_string();
        let profile = root.create_profile(OsStr::new(&nonce)).unwrap();
        drop(profile.create_file(LEASE).unwrap());
        let (pid, birth) = if alive {
            (
                std::process::id(),
                process_birth(std::process::id()).unwrap(),
            )
        } else {
            dead(path)
        };
        let mut receipt = json!({"version":1,"nonce":nonce,"ownerProcessId":pid,"ownerBirth":birth,"phase":phase,"rootIdentity":profile.root_id,"profileIdentity":profile.id});
        if phase == "Owned" {
            let (host_pid, host_birth) = dead(path);
            receipt["owned"] = json!(Owned {
                version: 1,
                nonce: nonce.clone(),
                root_identity: profile.root_id,
                profile_identity: profile.id,
                parent_pid: pid,
                parent_birth: birth,
                host_pid,
                host_birth,
                group: None,
                job: Some(format!("Local\\ClovyBrowser-{nonce}")),
                boot: boot_identity().unwrap()
            });
        }
        let mut file = profile.create_file(RECEIPT).unwrap();
        serde_json::to_writer(&mut file, &receipt).unwrap();
        native::sync(&file).unwrap();
        drop(file);
        if phase != "Reserved" {
            let mut marker = profile.create_file(MARKER).unwrap();
            marker.write_all(b"Starting\n").unwrap();
            native::sync(&marker).unwrap();
        }
        profile.sync().unwrap();
        (nonce, profile)
    }
    fn payload(path: &Path, nonce: &str) {
        let profile = path.join(nonce);
        std::fs::create_dir(profile.join("nested")).unwrap();
        for name in CONTROLS {
            std::fs::write(profile.join("nested").join(name), b"payload").unwrap();
        }
        std::fs::write(profile.join("payload"), b"synthetic").unwrap();
    }
    fn collector(path: &Path, crash: Option<&str>, pause: Option<&str>) -> Child {
        let mut cmd = command(path, "collect");
        if let Some(name) = crash {
            cmd.env("CLOVY_WIN_GC_CRASH", name);
        }
        if let Some(name) = pause {
            cmd.env("CLOVY_WIN_GC_PAUSE", name);
        }
        cmd.spawn().unwrap()
    }
    #[test]
    fn process_fixture() {
        let Some(path) = std::env::var_os("CLOVY_WIN_GC_CHILD") else {
            return;
        };
        match std::env::var("CLOVY_WIN_GC_ROLE").unwrap().as_str() {
            "wait" => {
                println!("BIRTH={}", process_birth(std::process::id()).unwrap());
                std::io::stdout().flush().unwrap();
                std::io::stdin().read_exact(&mut [0]).unwrap();
            }
            "owned" => {
                let mut line = String::new();
                std::io::stdin().read_line(&mut line).unwrap();
                let grant: super::super::clovy_browser_grant::Grant =
                    serde_json::from_str(&line).unwrap();
                // A private job containing only this subprocess exercises the
                // real host admission/publication, without assigning the suite.
                use std::{ffi::c_void, os::windows::ffi::OsStrExt};
                #[link(name = "kernel32")]
                extern "system" {
                    fn CreateJobObjectW(attrs: *const c_void, name: *const u16) -> *mut c_void;
                    fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
                    fn GetCurrentProcess() -> *mut c_void;
                    fn CloseHandle(handle: *mut c_void) -> i32;
                }
                let name = grant.browser_job_name();
                let wide: Vec<_> = OsStr::new(&name).encode_wide().chain(Some(0)).collect();
                let job = unsafe { CreateJobObjectW(std::ptr::null(), wide.as_ptr()) };
                assert!(!job.is_null());
                assert_ne!(
                    unsafe { AssignProcessToJobObject(job, GetCurrentProcess()) },
                    0
                );
                let lease = grant.acquire_owned(Some(&name)).unwrap();
                println!("OWNED");
                std::io::stdout().flush().unwrap();
                std::io::stdin().read_exact(&mut [0]).unwrap();
                drop(lease);
                assert_ne!(unsafe { CloseHandle(job) }, 0);
            }
            "collect" => {
                let diagnostics = RecoveryRoot::open(Path::new(&path))
                    .unwrap()
                    .recover()
                    .unwrap();
                assert!(diagnostics.is_empty(), "{diagnostics:?}");
            }
            other => panic!("unknown fixture role {other}"),
        }
    }
    #[test]
    fn publications_and_same_lease_cancelled_collection_retain_normal_directory_references() {
        let temp = tempfile::tempdir().unwrap();
        let root = RecoveryRoot::open(temp.path()).unwrap();
        let (nonce, profile) = fixture(temp.path(), "Reserved", true);
        let mut receipt = read_json(&mut profile.open_file(RECEIPT, false).unwrap()).unwrap();
        let mut marker = profile.create_file(MARKER).unwrap();
        marker.write_all(b"Starting\n").unwrap();
        native::sync(&marker).unwrap();
        receipt["phase"] = json!("Starting");
        profile
            .replace_receipt(&serde_json::to_vec(&receipt).unwrap())
            .unwrap();
        drop(marker);
        let lease = profile.open_file(LEASE, true).unwrap();
        lock_profile(&lease).unwrap();
        let id = native::identity(&lease).unwrap();
        let cleanup = profile
            .authorize(Some(lease))
            .unwrap()
            .cancel_starting()
            .unwrap();
        assert_eq!(cleanup.receipt["phase"], "Cancelled");
        assert_eq!(native::identity(&cleanup.lease).unwrap(), id);
        assert_eq!(
            read_json(&mut cleanup.profile.open_file(RECEIPT, false).unwrap()).unwrap(),
            cleanup.receipt
        );
        cleanup.delete().unwrap();
        assert!(!temp.path().join(nonce).exists());
        assert!(root.names().unwrap().is_empty());
    }
    #[test]
    fn real_owned_publication_and_stop_consume_the_same_captured_profile() {
        let temp = tempfile::tempdir().unwrap();
        let root = RecoveryRoot::open(temp.path()).unwrap();
        let (nonce, profile) = fixture(temp.path(), "Starting", true);
        payload(temp.path(), &nonce);
        let grant = super::super::clovy_browser_grant::Grant::new(&temp.path().join(&nonce), false)
            .unwrap();
        let mut host = command(temp.path(), "owned").spawn().unwrap();
        writeln!(
            host.stdin.as_mut().unwrap(),
            "{}",
            serde_json::to_string(&grant).unwrap()
        )
        .unwrap();
        let mut output = BufReader::new(host.stdout.take().unwrap());
        barrier(&mut output, "OWNED");
        assert_eq!(
            read_json(&mut profile.open_file(RECEIPT, false).unwrap()).unwrap()["phase"],
            "Owned"
        );
        assert!(!root.recover().unwrap().is_empty());
        host.stdin.take().unwrap().write_all(b"x").unwrap();
        assert!(host.wait().unwrap().success());
        profile.delete_stopped(None, &nonce).unwrap();
        assert!(root.names().unwrap().is_empty());
    }
    #[test]
    fn holder_publication_closes_descendant_reader_and_directory_delete_is_operational() {
        let temp = tempfile::tempdir().unwrap();
        let root = RecoveryRoot::open(temp.path()).unwrap();
        let holder = native::directory(&root.directory, OsStr::new("temporary"), true).unwrap();
        let mut receipt = native::file(&holder, OsStr::new(HOLDER_RECEIPT), true, true).unwrap();
        receipt.write_all(b"synthetic").unwrap();
        native::sync(&receipt).unwrap();
        drop(receipt);
        native::sync(&holder).unwrap();
        native::rename(
            &root.directory,
            OsStr::new("temporary"),
            &holder,
            &root.directory,
            OsStr::new("published"),
            false,
        )
        .unwrap();
        let receipt = native::file(&holder, OsStr::new(HOLDER_RECEIPT), true, false).unwrap();
        native::remove(&holder, OsStr::new(HOLDER_RECEIPT), &receipt, false).unwrap();
        drop(receipt);
        native::remove_directory(&root.directory, OsStr::new("published"), &holder).unwrap();
        drop(holder);
        assert!(root.names().unwrap().is_empty());
    }
    #[test]
    fn pinned_profile_blocks_swap_and_payload_delete_share_three_does_not_reopen() {
        let temp = tempfile::tempdir().unwrap();
        let (nonce, profile) = fixture(temp.path(), "Starting", true);
        payload(temp.path(), &nonce);
        let original = temp.path().join(&nonce);
        let target = temp.path().join("swapped");
        HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |at| {
                if at == "quarantined" {
                    assert!(std::fs::rename(&original, &target).is_err());
                    assert!(original.join(LEASE).is_file());
                }
            }))
        });
        let result = profile.delete_stopped(None, &nonce);
        HOOK.with(|hook| *hook.borrow_mut() = None);
        result.unwrap();
        assert!(std::fs::read_dir(temp.path()).unwrap().next().is_none());
    }
    #[test]
    fn crash_boundaries_resume_original_pinned_location_without_recreating_lease() {
        for phase in ["Reserved", "Starting", "Owned", "Cancelled"] {
            for boundary in [
                "authorized",
                "quarantined",
                "payload-entry",
                "payload-gone",
                "terminal-published",
                "removed-.clovy-reserved.json",
                "removed-.clovy-lease",
                "profile-retiring",
                "pin-closed",
                "profile-removed",
            ]
            .into_iter()
            .chain((phase == "Starting").then_some("receipt-reader-closed"))
            .chain((phase == "Starting").then_some("cancelled"))
            .chain((phase != "Reserved").then_some("removed-.clovy-starting"))
            {
                let temp = tempfile::tempdir().unwrap();
                let (nonce, profile) = fixture(temp.path(), phase, false);
                drop(profile);
                payload(temp.path(), &nonce);
                let output = collector(temp.path(), Some(boundary), None)
                    .wait_with_output()
                    .unwrap();
                assert_eq!(
                    output.status.code(),
                    Some(77),
                    "{phase} {boundary}: {}",
                    String::from_utf8_lossy(&output.stdout)
                );
                if boundary == "terminal-published" {
                    let terminal: Terminal = serde_json::from_slice(
                        &std::fs::read(temp.path().join(terminal_name(&nonce))).unwrap(),
                    )
                    .unwrap();
                    assert!(matches!(
                        terminal.location,
                        TerminalLocation::WindowsPinnedOriginal
                    ));
                    assert!(temp.path().join(&nonce).is_dir());
                }
                let lease_missing = !temp.path().join(&nonce).join(LEASE).exists();
                assert!(
                    collector(temp.path(), None, None).wait().unwrap().success(),
                    "{phase} {boundary}"
                );
                assert!(
                    std::fs::read_dir(temp.path()).unwrap().next().is_none(),
                    "{phase} {boundary}"
                );
                if lease_missing {
                    assert!(!temp.path().join(nonce).join(LEASE).exists());
                }
            }
        }
    }
    #[test]
    fn concurrent_collectors_exclude_across_terminal_and_pin_release() {
        for boundary in [
            "authorized",
            "terminal-published",
            "removed-.clovy-lease",
            "pin-closed",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let (nonce, profile) = fixture(temp.path(), "Starting", false);
            drop(profile);
            let mut first = collector(temp.path(), None, Some(boundary));
            let mut output = BufReader::new(first.stdout.take().unwrap());
            barrier(&mut output, "GC_BARRIER");
            assert!(!RecoveryRoot::open(temp.path())
                .unwrap()
                .recover()
                .unwrap()
                .is_empty());
            assert!(temp.path().join(&nonce).exists());
            first.stdin.take().unwrap().write_all(b"x").unwrap();
            assert!(first.wait().unwrap().success());
            assert!(std::fs::read_dir(temp.path()).unwrap().next().is_none());
        }
    }
    #[test]
    fn pin_release_under_terminal_preserves_a_replacement_directory() {
        let temp = tempfile::tempdir().unwrap();
        let (nonce, profile) = fixture(temp.path(), "Starting", true);
        let original = temp.path().join(&nonce);
        let saved = temp.path().join("saved");
        let replacement = original.clone();
        HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |at| {
                if at == "pin-closed" {
                    std::fs::rename(&original, &saved).unwrap();
                    std::fs::create_dir(&original).unwrap();
                    std::fs::write(original.join("sentinel"), b"keep").unwrap();
                }
            }))
        });
        let result = profile.delete_stopped(None, &nonce);
        HOOK.with(|hook| *hook.borrow_mut() = None);
        assert!(result.is_err());
        assert_eq!(
            std::fs::read(replacement.join("sentinel")).unwrap(),
            b"keep"
        );
        assert!(temp.path().join(terminal_name(&nonce)).exists());
        assert!(!RecoveryRoot::open(temp.path())
            .unwrap()
            .recover()
            .unwrap()
            .is_empty());
        assert_eq!(
            std::fs::read(replacement.join("sentinel")).unwrap(),
            b"keep"
        );
    }
    #[test]
    fn terminal_retains_unexpected_payload_replaced_controls_and_legacy_records() {
        for change in [
            "payload",
            RECEIPT,
            LEASE,
            MARKER,
            "legacy",
            "ambiguous",
            "profile",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let (nonce, profile) = fixture(temp.path(), "Starting", false);
            drop(profile);
            assert_eq!(
                collector(temp.path(), Some("terminal-published"), None)
                    .wait()
                    .unwrap()
                    .code(),
                Some(77)
            );
            let directory = temp.path().join(&nonce);
            let record = temp.path().join(terminal_name(&nonce));
            match change {
                "legacy" | "ambiguous" => {
                    let mut value: Value =
                        serde_json::from_slice(&std::fs::read(&record).unwrap()).unwrap();
                    if change == "legacy" {
                        value["version"] = json!(1);
                    } else {
                        value.as_object_mut().unwrap().remove("location");
                    }
                    std::fs::write(&record, serde_json::to_vec(&value).unwrap()).unwrap();
                }
                "payload" => {
                    std::fs::write(directory.join("unexpected"), b"keep").unwrap();
                }
                "profile" => {
                    std::fs::rename(&directory, temp.path().join("saved")).unwrap();
                    std::fs::create_dir(&directory).unwrap();
                    std::fs::write(directory.join("sentinel"), b"keep").unwrap();
                }
                control => {
                    let path = directory.join(control);
                    let bytes = std::fs::read(&path).unwrap();
                    std::fs::rename(&path, temp.path().join("saved")).unwrap();
                    std::fs::write(path, bytes).unwrap();
                }
            }
            assert!(
                !RecoveryRoot::open(temp.path())
                    .unwrap()
                    .recover()
                    .unwrap()
                    .is_empty(),
                "{change}"
            );
            assert!(record.exists());
            assert!(directory.is_dir());
            if change == "payload" {
                assert_eq!(
                    std::fs::read(directory.join("unexpected")).unwrap(),
                    b"keep"
                );
            }
            if change == "profile" {
                assert_eq!(std::fs::read(directory.join("sentinel")).unwrap(), b"keep");
            }
        }
    }
}

#[cfg(unix)]
mod native {
    use super::*;
    use std::{
        ffi::{CStr, CString},
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::{
                ffi::{OsStrExt, OsStringExt},
                fs::MetadataExt,
            },
        },
        path::Component,
    };
    fn part(name: &OsStr) -> io::Result<CString> {
        let bytes = name.as_bytes();
        if bytes.is_empty() || bytes == b"." || bytes == b".." || bytes.contains(&b'/') {
            return Err(invalid());
        }
        CString::new(bytes).map_err(io::Error::other)
    }
    pub(super) fn identity(file: &File) -> io::Result<Id> {
        let m = file.metadata()?;
        Ok([m.dev(), m.ino()])
    }
    fn open(parent: &File, name: &OsStr, flags: i32) -> io::Result<File> {
        let name = part(name)?;
        let nofollow = libc::O_NOFOLLOW;
        #[cfg(target_os = "macos")]
        let nofollow = if flags & libc::O_SYMLINK != 0 {
            // O_SYMLINK opens the link itself; adding O_NOFOLLOW makes Darwin reject it.
            0
        } else {
            nofollow
        };
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                flags | nofollow | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }
    pub(super) fn directory(parent: &File, name: &OsStr, create: bool) -> io::Result<File> {
        if create && unsafe { libc::mkdirat(parent.as_raw_fd(), part(name)?.as_ptr(), 0o700) } != 0
        {
            return Err(io::Error::last_os_error());
        }
        open(parent, name, libc::O_RDONLY | libc::O_DIRECTORY)
    }
    pub(super) fn file(parent: &File, name: &OsStr, write: bool, create: bool) -> io::Result<File> {
        let f = open(
            parent,
            name,
            libc::O_NONBLOCK
                | if create {
                    libc::O_RDWR | libc::O_CREAT | libc::O_EXCL
                } else if write {
                    libc::O_RDWR
                } else {
                    libc::O_RDONLY
                },
        )?;
        if !f.metadata()?.is_file() {
            return Err(invalid());
        }
        Ok(f)
    }
    pub(super) fn root(path: &Path, create: bool) -> io::Result<File> {
        if !path.is_absolute() {
            return Err(invalid());
        }
        let mut dir = File::open("/")?;
        for component in path.components() {
            match component {
                Component::RootDir => (),
                Component::Normal(name) => {
                    dir = match directory(&dir, name, false) {
                        Ok(next) => next,
                        Err(e) if create && e.kind() == io::ErrorKind::NotFound => {
                            let next = match directory(&dir, name, true) {
                                Ok(next) => next,
                                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                                    directory(&dir, name, false)?
                                }
                                Err(e) => return Err(e),
                            };
                            sync(&dir)?;
                            next
                        }
                        Err(e) => return Err(e),
                    };
                }
                _ => return Err(invalid()),
            }
        }
        Ok(dir)
    }
    fn at(parent: &File, name: &OsStr) -> io::Result<Entry> {
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe {
            libc::fstatat(
                parent.as_raw_fd(),
                part(name)?.as_ptr(),
                &mut stat,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Entry {
            name: name.to_owned(),
            id: [stat.st_dev as u64, stat.st_ino as u64],
            directory: stat.st_mode & libc::S_IFMT == libc::S_IFDIR,
            link: stat.st_mode & libc::S_IFMT == libc::S_IFLNK,
        })
    }
    pub(super) fn same(parent: &File, name: &OsStr, file: &File) -> io::Result<()> {
        if at(parent, name)?.id != identity(file)? {
            return Err(invalid());
        }
        Ok(())
    }
    pub(super) fn list(dir: &File) -> io::Result<Vec<Entry>> {
        let fd = unsafe { libc::dup(dir.as_raw_fd()) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let stream = unsafe { libc::fdopendir(fd) };
        if stream.is_null() {
            unsafe {
                libc::close(fd);
            }
            return Err(io::Error::last_os_error());
        }
        unsafe {
            libc::rewinddir(stream);
        }
        let result = (|| {
            let mut entries = Vec::new();
            loop {
                #[cfg(target_os = "linux")]
                unsafe {
                    *libc::__errno_location() = 0;
                }
                #[cfg(target_os = "macos")]
                unsafe {
                    *libc::__error() = 0;
                }
                let entry = unsafe { libc::readdir(stream) };
                if entry.is_null() {
                    let e = io::Error::last_os_error();
                    if e.raw_os_error().unwrap_or(0) != 0 {
                        return Err(e);
                    }
                    break;
                }
                let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
                if name != b"." && name != b".." {
                    entries.push(at(dir, &OsString::from_vec(name.to_vec()))?);
                }
            }
            Ok(entries)
        })();
        unsafe {
            libc::closedir(stream);
        }
        result
    }
    pub(super) fn entry(parent: &File, e: &Entry, _payload: bool) -> io::Result<File> {
        // Linux O_PATH opens the symlink itself; Darwin O_SYMLINK does likewise.
        #[cfg(target_os = "linux")]
        let flags = if e.link {
            libc::O_PATH
        } else {
            libc::O_RDONLY | libc::O_NONBLOCK | if e.directory { libc::O_DIRECTORY } else { 0 }
        };
        #[cfg(target_os = "macos")]
        let flags = if e.link {
            libc::O_SYMLINK
        } else {
            libc::O_RDONLY | libc::O_NONBLOCK | if e.directory { libc::O_DIRECTORY } else { 0 }
        };
        let f = open(parent, &e.name, flags)?;
        if identity(&f)? != e.id {
            return Err(invalid());
        }
        Ok(f)
    }
    pub(super) fn sync(file: &File) -> io::Result<()> {
        file.sync_all()
    }
    pub(super) fn remove(
        parent: &File,
        name: &OsStr,
        file: &File,
        directory: bool,
    ) -> io::Result<()> {
        same(parent, name, file)?;
        if unsafe {
            libc::unlinkat(
                parent.as_raw_fd(),
                part(name)?.as_ptr(),
                if directory { libc::AT_REMOVEDIR } else { 0 },
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    pub(super) fn remove_directory(parent: &File, name: &OsStr, file: &File) -> io::Result<()> {
        remove(parent, name, file, true)
    }
    pub(super) fn rename(
        parent: &File,
        name: &OsStr,
        file: &File,
        target: &File,
        to: &OsStr,
        replace: bool,
    ) -> io::Result<()> {
        same(parent, name, file)?;
        let (from, to) = (part(name)?, part(to)?);
        let result = unsafe {
            if replace {
                libc::renameat(
                    parent.as_raw_fd(),
                    from.as_ptr(),
                    target.as_raw_fd(),
                    to.as_ptr(),
                )
            } else {
                #[cfg(target_os = "linux")]
                {
                    libc::renameat2(
                        parent.as_raw_fd(),
                        from.as_ptr(),
                        target.as_raw_fd(),
                        to.as_ptr(),
                        libc::RENAME_NOREPLACE,
                    )
                }
                #[cfg(target_os = "macos")]
                {
                    libc::renameatx_np(
                        parent.as_raw_fd(),
                        from.as_ptr(),
                        target.as_raw_fd(),
                        to.as_ptr(),
                        libc::RENAME_EXCL,
                    )
                }
            }
        };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
        same(target, OsStr::from_bytes(to.to_bytes()), file)
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::{
        ffi::c_void,
        os::windows::{
            ffi::{OsStrExt, OsStringExt},
            fs::OpenOptionsExt,
            io::{AsRawHandle, FromRawHandle},
        },
        path::{Component, PathBuf},
        ptr,
    };
    #[repr(C)]
    struct UnicodeString {
        length: u16,
        maximum_length: u16,
        buffer: *mut u16,
    }
    #[repr(C)]
    struct ObjectAttributes {
        length: u32,
        root: *mut c_void,
        name: *mut UnicodeString,
        attributes: u32,
        security: *mut c_void,
        quality: *mut c_void,
    }
    #[repr(C)]
    struct IoStatus {
        status: usize,
        information: usize,
    }
    #[repr(C)]
    #[derive(Default)]
    struct FileInfo {
        attributes: u32,
        created: [u32; 2],
        accessed: [u32; 2],
        written: [u32; 2],
        volume: u32,
        size_high: u32,
        size_low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }
    #[repr(C)]
    struct DirectoryInfo {
        next: u32,
        index: u32,
        creation: i64,
        access: i64,
        write: i64,
        change: i64,
        size: i64,
        allocation: i64,
        attributes: u32,
        name_len: u32,
        ea_size: u32,
        short_len: u8,
        short_name: [u16; 12],
        file_id: i64,
        file_name: [u16; 1],
    }
    #[repr(C)]
    struct RenameInfo {
        replace: u32,
        root: *mut c_void,
        name_len: u32,
        name: [u16; 1],
    }
    #[link(name = "ntdll")]
    extern "system" {
        fn NtCreateFile(
            handle: *mut *mut c_void,
            access: u32,
            attributes: *mut ObjectAttributes,
            status: *mut IoStatus,
            allocation: *const i64,
            file_attributes: u32,
            share: u32,
            disposition: u32,
            options: u32,
            ea: *mut c_void,
            ea_length: u32,
        ) -> i32;
        fn NtFlushBuffersFileEx(
            file: *mut c_void,
            flags: u32,
            parameters: *mut c_void,
            size: u32,
            status: *mut IoStatus,
        ) -> i32;
        fn RtlNtStatusToDosError(status: i32) -> u32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetFileInformationByHandle(handle: *mut c_void, info: *mut FileInfo) -> i32;
        fn GetFileInformationByHandleEx(
            handle: *mut c_void,
            class: i32,
            info: *mut c_void,
            size: u32,
        ) -> i32;
        fn SetFileInformationByHandle(
            handle: *mut c_void,
            class: i32,
            info: *const c_void,
            size: u32,
        ) -> i32;
        fn ReOpenFile(handle: *mut c_void, access: u32, share: u32, flags: u32) -> *mut c_void;
    }
    fn nt_error(code: i32) -> io::Error {
        io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(code) } as i32)
    }
    fn info(file: &File) -> io::Result<FileInfo> {
        let mut value = FileInfo::default();
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut value) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(value)
    }
    pub(super) fn identity(file: &File) -> io::Result<Id> {
        let i = info(file)?;
        Ok([
            i.volume as u64,
            (u64::from(i.index_high) << 32) | u64::from(i.index_low),
        ])
    }
    fn component(name: &OsStr) -> io::Result<Vec<u16>> {
        let wide: Vec<u16> = name.encode_wide().collect();
        if wide.is_empty()
            || wide == [46]
            || wide == [46, 46]
            || wide.iter().any(|c| matches!(*c, 0 | 47 | 92 | 58))
        {
            return Err(invalid());
        }
        Ok(wide)
    }
    fn open(
        parent: &File,
        name: &OsStr,
        directory: Option<bool>,
        create: bool,
        access: u32,
        reparse: bool,
    ) -> io::Result<File> {
        open_shared(parent, name, directory, create, access, reparse, 7)
    }
    fn open_shared(
        parent: &File,
        name: &OsStr,
        directory: Option<bool>,
        create: bool,
        access: u32,
        reparse: bool,
        share: u32,
    ) -> io::Result<File> {
        let mut wide = component(name)?;
        let length = u16::try_from(wide.len() * 2).map_err(io::Error::other)?;
        let mut name = UnicodeString {
            length,
            maximum_length: length,
            buffer: wide.as_mut_ptr(),
        };
        let mut attrs = ObjectAttributes {
            length: std::mem::size_of::<ObjectAttributes>() as u32,
            root: parent.as_raw_handle(),
            name: &mut name,
            attributes: 0x40,
            security: ptr::null_mut(),
            quality: ptr::null_mut(),
        };
        let mut status = IoStatus {
            status: 0,
            information: 0,
        };
        let mut handle = ptr::null_mut();
        let options = 0x00200020
            | if create { 2 } else { 0 }
            | match directory {
                Some(true) => 1,
                Some(false) => 0x40,
                None => 0,
            };
        // Controls require DELETE sharing for their independently retained
        // handles. Payload entries deny it and are deleted through this handle.
        let code = unsafe {
            NtCreateFile(
                &mut handle,
                access,
                &mut attrs,
                &mut status,
                ptr::null(),
                0,
                share,
                if create { 2 } else { 1 },
                options,
                ptr::null_mut(),
                0,
            )
        };
        if code < 0 {
            return Err(nt_error(code));
        }
        let file = unsafe { File::from_raw_handle(handle) };
        let attrs = info(&file)?.attributes;
        if (!reparse && attrs & 0x400 != 0) || directory.is_some_and(|d| d != (attrs & 0x10 != 0)) {
            return Err(invalid());
        }
        Ok(file)
    }
    fn reopen(file: &File, access: u32) -> io::Result<File> {
        reopen_shared(file, access, 7)
    }
    fn reopen_shared(file: &File, access: u32, share: u32) -> io::Result<File> {
        let handle =
            unsafe { ReOpenFile(file.as_raw_handle(), access, share, 0x02000000 | 0x00200000) };
        if handle as isize == -1 {
            return Err(io::Error::last_os_error());
        }
        let next = unsafe { File::from_raw_handle(handle) };
        if identity(file)? != identity(&next)? || info(&next)?.attributes & 0x400 != 0 {
            return Err(invalid());
        }
        Ok(next)
    }
    pub(super) fn directory(parent: &File, name: &OsStr, create: bool) -> io::Result<File> {
        // A retained rename destination must not request DELETE (MS-FSA 185).
        open(parent, name, Some(true), create, 0x001000a7, false)
    }
    pub(super) fn pin_directory(file: &File) -> io::Result<File> {
        let pin = reopen_shared(file, 0x001000a7, 3)?;
        if info(&pin)?.attributes & 0x10 == 0 {
            return Err(invalid());
        }
        Ok(pin)
    }
    pub(super) fn unpin_directory(pin: File) -> io::Result<File> {
        let reference = reopen(&pin, 0x001000a7)?;
        drop(pin);
        Ok(reference)
    }
    pub(super) fn file(parent: &File, name: &OsStr, write: bool, create: bool) -> io::Result<File> {
        open(
            parent,
            name,
            Some(false),
            create,
            0x00110081 | if write || create { 2 } else { 0 },
            false,
        )
    }
    pub(super) fn root(path: &Path, create: bool) -> io::Result<File> {
        if !path.is_absolute() {
            return Err(invalid());
        }
        let mut parts = path.components();
        let Some(Component::Prefix(prefix)) = parts.next() else {
            return Err(invalid());
        };
        if !matches!(parts.next(), Some(Component::RootDir)) {
            return Err(invalid());
        }
        let mut base = PathBuf::from(prefix.as_os_str());
        base.push("\\");
        // Existing ancestors need traversal and attributes only, never listing
        // or write permissions just to reach the recovery root.
        let mut dir = std::fs::OpenOptions::new()
            .access_mode(0x001000a0)
            .share_mode(7)
            .custom_flags(0x02000000 | 0x00200000)
            .open(base)?;
        if info(&dir)?.attributes & 0x400 != 0 {
            return Err(invalid());
        }
        let mut parts = parts.peekable();
        while let Some(part) = parts.next() {
            let Component::Normal(name) = part else {
                return Err(invalid());
            };
            let access = if parts.peek().is_none() {
                0x001000a7
            } else {
                0x001000a0
            };
            dir = match open(&dir, name, Some(true), false, access, false) {
                Ok(next) => next,
                Err(e) if create && e.kind() == io::ErrorKind::NotFound => {
                    let parent = reopen(&dir, 0x001000a7)?;
                    let next = match open(&parent, name, Some(true), true, access | 2, false) {
                        Ok(next) => next,
                        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                            open(&parent, name, Some(true), false, access, false)?
                        }
                        Err(e) => return Err(e),
                    };
                    sync(&parent)?;
                    next
                }
                Err(e) => return Err(e),
            };
        }
        Ok(dir)
    }
    pub(super) fn sync(file: &File) -> io::Result<()> {
        // NtFlushBuffersFileEx(flags=0) requires FILE_WRITE_DATA or
        // FILE_APPEND_DATA, granted on every handle synchronized here. Unlike
        // File::sync_all/FlushFileBuffers it does not require GENERIC_WRITE.
        let mut status = IoStatus {
            status: 0,
            information: 0,
        };
        let result = unsafe {
            NtFlushBuffersFileEx(file.as_raw_handle(), 0, ptr::null_mut(), 0, &mut status)
        };
        if result < 0 {
            return Err(nt_error(result));
        }
        Ok(())
    }
    pub(super) fn list(dir: &File) -> io::Result<Vec<Entry>> {
        let mut entries = Vec::new();
        let mut first = true;
        let volume = identity(dir)?[0];
        loop {
            let mut storage = vec![0u64; 8192];
            let result = unsafe {
                GetFileInformationByHandleEx(
                    dir.as_raw_handle(),
                    if first { 11 } else { 10 },
                    storage.as_mut_ptr().cast(),
                    (storage.len() * 8) as u32,
                )
            };
            first = false;
            if result == 0 {
                let e = io::Error::last_os_error();
                if e.raw_os_error() == Some(18) {
                    return Ok(entries);
                }
                return Err(e);
            }
            let size = storage.len() * 8;
            let mut offset = 0;
            loop {
                let fixed = std::mem::offset_of!(DirectoryInfo, file_name);
                if offset + fixed > size {
                    return Err(invalid());
                }
                let data = unsafe {
                    &*storage
                        .as_ptr()
                        .cast::<u8>()
                        .add(offset)
                        .cast::<DirectoryInfo>()
                };
                let count = data.name_len as usize;
                if count % 2 != 0 || offset + fixed + count > size {
                    return Err(invalid());
                }
                let name = unsafe {
                    std::slice::from_raw_parts(
                        ptr::addr_of!(data.file_name).cast::<u16>(),
                        count / 2,
                    )
                };
                if name != [46] && name != [46, 46] {
                    entries.push(Entry {
                        name: OsString::from_wide(name),
                        id: [volume, data.file_id as u64],
                        directory: data.attributes & 0x10 != 0,
                        link: data.attributes & 0x400 != 0,
                    });
                }
                if data.next == 0 {
                    break;
                }
                let next = data.next as usize;
                if next < fixed || offset + next >= size || next % 8 != 0 {
                    return Err(invalid());
                }
                offset += next;
            }
        }
    }
    pub(super) fn entry(parent: &File, e: &Entry, payload: bool) -> io::Result<File> {
        let file = open_shared(
            parent,
            &e.name,
            None,
            false,
            if e.directory && !e.link {
                0x001100a7
            } else {
                0x00110083
            },
            true,
            if payload { 3 } else { 7 },
        )?;
        if identity(&file)? != e.id {
            return Err(invalid());
        }
        Ok(file)
    }
    pub(super) fn same(parent: &File, name: &OsStr, file: &File) -> io::Result<()> {
        let current = open(parent, name, None, false, 0x00100080, true)?;
        if identity(&current)? != identity(file)? {
            return Err(invalid());
        }
        Ok(())
    }
    pub(super) fn remove(
        parent: &File,
        name: &OsStr,
        file: &File,
        _directory: bool,
    ) -> io::Result<()> {
        same(parent, name, file)?;
        let delete: u8 = 1;
        if unsafe {
            SetFileInformationByHandle(file.as_raw_handle(), 4, ptr::from_ref(&delete).cast(), 1)
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    pub(super) fn remove_directory(parent: &File, name: &OsStr, file: &File) -> io::Result<()> {
        same(parent, name, file)?;
        if !list(file)?.is_empty() {
            return Err(invalid());
        }
        // Only directory references reach this operation. Payload handles
        // already own DELETE with share=3 and must never be reopened for it.
        let destructive = reopen(file, 0x001100a7)?;
        remove(parent, name, &destructive, true)
    }
    pub(super) fn rename(
        parent: &File,
        name: &OsStr,
        file: &File,
        target: &File,
        to: &OsStr,
        replace: bool,
    ) -> io::Result<()> {
        same(parent, name, file)?;
        let destructive;
        let source = if info(file)?.attributes & 0x10 != 0 {
            destructive = reopen(file, 0x001100a7)?;
            &destructive
        } else {
            file
        };
        let name = component(to)?;
        let size = std::mem::offset_of!(RenameInfo, name) + name.len() * 2;
        let mut storage = vec![0u64; size.div_ceil(8)];
        let data = storage.as_mut_ptr().cast::<RenameInfo>();
        unsafe {
            (*data).replace = u32::from(replace);
            (*data).root = target.as_raw_handle();
            (*data).name_len = (name.len() * 2) as u32;
            ptr::copy_nonoverlapping(
                name.as_ptr(),
                ptr::addr_of_mut!((*data).name).cast::<u16>(),
                name.len(),
            );
            if SetFileInformationByHandle(source.as_raw_handle(), 3, data.cast(), size as u32) == 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        same(target, to, file)
    }
    pub(super) fn job_exited(name: &str) -> io::Result<bool> {
        #[repr(C)]
        struct Accounting {
            times: [i64; 4],
            faults: u32,
            processes: u32,
            active: u32,
            terminated: u32,
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn OpenJobObjectW(access: u32, inherit: i32, name: *const u16) -> *mut c_void;
            fn QueryInformationJobObject(
                job: *mut c_void,
                class: u32,
                data: *mut c_void,
                size: u32,
                returned: *mut u32,
            ) -> i32;
            fn CloseHandle(handle: *mut c_void) -> i32;
        }
        let wide: Vec<u16> = OsStr::new(name).encode_wide().chain(Some(0)).collect();
        let handle = unsafe { OpenJobObjectW(4, 0, wide.as_ptr()) };
        if handle.is_null() {
            let e = io::Error::last_os_error();
            return if e.raw_os_error() == Some(2) {
                Ok(true)
            } else {
                Err(e)
            };
        }
        let mut data: Accounting = unsafe { std::mem::zeroed() };
        let result = unsafe {
            QueryInformationJobObject(
                handle,
                1,
                ptr::from_mut(&mut data).cast(),
                std::mem::size_of_val(&data) as u32,
                ptr::null_mut(),
            )
        };
        let error = io::Error::last_os_error();
        unsafe {
            CloseHandle(handle);
        }
        if result == 0 {
            return Err(error);
        }
        Ok(data.active == 0)
    }
}
