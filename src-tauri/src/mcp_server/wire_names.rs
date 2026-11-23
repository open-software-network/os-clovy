//! Passive, turn-local names for the private native MCP transports. Canonical
//! identities remain in the existing catalog, dispatcher, approvals and history.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde_json::Value;
use sha2::{Digest, Sha256};

const PREFIX: &str = "_c_";
const COMMON_BYTES: usize = 36;
const ENCODED_BYTES: usize = 35;

#[derive(Default)]
struct Names {
    canonical: HashMap<Arc<str>, Arc<str>>,
    wire: HashMap<Arc<str>, Arc<str>>,
}

/// Clones share only this turn's published mappings. Equality and debugging
/// describe instance identity, never mutable contents or canonical names.
#[derive(Clone, Default)]
pub struct WireNames(Arc<Mutex<Names>>);

impl PartialEq for WireNames {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for WireNames {}

impl fmt::Debug for WireNames {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("WireNames")
            .field(&Arc::as_ptr(&self.0))
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireNameError {
    InvalidDescriptor,
    Collision,
    Unavailable,
}

impl fmt::Display for WireNameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidDescriptor => "Invalid tool descriptor in the private MCP catalog.",
            Self::Collision => "Conflicting tool names in the private MCP catalog.",
            Self::Unavailable => "Private MCP tool names are unavailable.",
        })
    }
}

impl std::error::Error for WireNameError {}

/// Common names borrow the request. Encoded names retain the full canonical
/// allocation across asynchronous dispatch without holding the map lock.
pub enum ResolvedName<'a> {
    Borrowed(&'a str),
    Canonical(Arc<str>),
}

impl AsRef<str> for ResolvedName<'_> {
    fn as_ref(&self) -> &str {
        match self {
            Self::Borrowed(name) => name,
            Self::Canonical(name) => name,
        }
    }
}

fn common(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= COMMON_BYTES
        && !name.starts_with(PREFIX)
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn encode(name: &str) -> Arc<str> {
    let digest = Sha256::digest(name.as_bytes());
    let mut wire = String::with_capacity(ENCODED_BYTES);
    wire.push_str(PREFIX);
    URL_SAFE_NO_PAD.encode_string(&digest[..24], &mut wire);
    Arc::from(wire)
}

impl WireNames {
    /// Publish the existing catalog atomically. A failed batch neither changes
    /// descriptors nor makes any of its new encoded names callable. Existing
    /// registrations are reused on subsequent discovery in this same turn.
    pub fn publish(&self, descriptors: &mut [Value]) -> Result<(), WireNameError> {
        let mut transformed = 0;
        for descriptor in descriptors.iter() {
            let name = descriptor
                .get("name")
                .and_then(Value::as_str)
                .ok_or(WireNameError::InvalidDescriptor)?;
            if !common(name) {
                transformed += 1;
            }
        }
        if transformed == 0 {
            return Ok(());
        }

        let mut names = self.0.lock().map_err(|_| WireNameError::Unavailable)?;
        let mut pending = Names::default();
        let mut replacements = Vec::with_capacity(transformed);
        for (index, descriptor) in descriptors.iter().enumerate() {
            let canonical = descriptor["name"]
                .as_str()
                .ok_or(WireNameError::InvalidDescriptor)?;
            if common(canonical) {
                continue;
            }
            let (canonical, wire) = match names
                .canonical
                .get_key_value(canonical)
                .or_else(|| pending.canonical.get_key_value(canonical))
            {
                Some((canonical, wire)) => (Arc::clone(canonical), Arc::clone(wire)),
                None => {
                    let wire = encode(canonical);
                    // Check both committed and staged names before inserting
                    // either direction. Never overwrite a different identity.
                    if names
                        .wire
                        .get(wire.as_ref())
                        .or_else(|| pending.wire.get(wire.as_ref()))
                        .is_some_and(|existing| existing.as_ref() != canonical)
                    {
                        return Err(WireNameError::Collision);
                    }
                    let canonical: Arc<str> = Arc::from(canonical);
                    pending
                        .canonical
                        .insert(Arc::clone(&canonical), Arc::clone(&wire));
                    pending
                        .wire
                        .insert(Arc::clone(&wire), Arc::clone(&canonical));
                    (canonical, wire)
                }
            };
            replacements.push((index, canonical, wire));
        }

        names.canonical.extend(pending.canonical);
        names.wire.extend(pending.wire);
        drop(names);
        for (index, canonical, wire) in replacements {
            let descriptor = &mut descriptors[index];
            let original = descriptor
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let description = if original.is_empty() {
                format!("Canonical tool: {canonical}")
            } else {
                format!("Canonical tool: {canonical}\n\n{original}")
            };
            descriptor["name"] = Value::String(wire.to_string());
            descriptor["description"] = Value::String(description);
        }
        Ok(())
    }

    /// Decode exactly once. Raw names requiring encoding and unpublished
    /// aliases are refused; this table is metadata, never execution authority.
    pub fn resolve<'a>(&self, name: &'a str) -> Option<ResolvedName<'a>> {
        if common(name) {
            return Some(ResolvedName::Borrowed(name));
        }
        if !name.starts_with(PREFIX) {
            return None;
        }
        let names = self.0.lock().ok()?;
        names
            .wire
            .get(name)
            .map(|canonical| ResolvedName::Canonical(Arc::clone(canonical)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn descriptor(name: &str) -> Value {
        json!({"name":name,"description":"Original semantics", "inputSchema":{"type":"object","properties":{"text":{"type":"string"}}}, "annotations":{"readOnlyHint":false}})
    }

    #[test]
    fn native_boundary_preserves_every_full_identity() {
        let names = WireNames::default();
        let canonical = [
            "a".repeat(36),
            "b".repeat(37),
            format!("mcp_external_{}", "remote".repeat(19) + "x"),
            "remote.tool/ação".to_string(),
            "_c_reserved".to_string(),
        ];
        assert_eq!(canonical[2].len(), 128);
        let mut published: Vec<_> = canonical.iter().map(|name| descriptor(name)).collect();
        names.publish(&mut published).unwrap();
        assert_eq!(published[0]["name"], canonical[0]);
        for (index, tool) in published.iter().enumerate() {
            let wire = tool["name"].as_str().unwrap();
            let qualified = format!("mcp_c0123456789012345678901_{wire}");
            assert!(qualified.len() <= 64);
            assert!(qualified
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte)));
            assert_eq!(names.resolve(wire).unwrap().as_ref(), canonical[index]);
            if index > 0 {
                assert_ne!(wire, canonical[index]);
                assert!(names.resolve(&canonical[index]).is_none());
            }
        }
    }

    #[test]
    fn clones_share_registration_and_fresh_turns_refuse_prior_aliases() {
        let names = WireNames::default();
        let clone = names.clone();
        let fresh = WireNames::default();
        assert_eq!(names, clone);
        assert_ne!(names, fresh);
        let canonical = "long_canonical_name_that_exceeds_thirty_six_bytes";
        let mut published = vec![descriptor(canonical)];
        names.publish(&mut published).unwrap();
        let wire = published[0]["name"].as_str().unwrap();
        let first = clone.resolve(wire).unwrap();
        assert_eq!(first.as_ref(), canonical);
        assert!(fresh.resolve(wire).is_none());
        assert!(names.resolve(canonical).is_none());
        assert!(!format!("{names:?}").contains(canonical));
        let mut repeated = vec![descriptor(canonical)];
        clone.publish(&mut repeated).unwrap();
        assert_eq!(repeated[0]["name"].as_str(), Some(wire));
    }

    #[test]
    fn reserved_canonical_names_are_decoded_once_without_recursion() {
        let names = WireNames::default();
        let mut first = vec![descriptor(&"x".repeat(37))];
        names.publish(&mut first).unwrap();
        let first_wire = first[0]["name"].as_str().unwrap();
        let mut second = vec![descriptor(first_wire)];
        names.publish(&mut second).unwrap();
        let second_wire = second[0]["name"].as_str().unwrap();
        assert_ne!(first_wire, second_wire);
        assert_eq!(names.resolve(second_wire).unwrap().as_ref(), first_wire);
        assert_eq!(names.resolve(first_wire).unwrap().as_ref(), "x".repeat(37));
    }

    #[test]
    fn invalid_batch_leaves_descriptors_and_new_registration_unpublished() {
        let names = WireNames::default();
        let canonical = "unpublished_name_because_the_whole_batch_is_invalid";
        let mut batch = vec![descriptor(canonical), json!({"name":42})];
        let before = batch.clone();
        assert_eq!(
            names.publish(&mut batch),
            Err(WireNameError::InvalidDescriptor)
        );
        assert_eq!(batch, before);
        assert!(names.resolve(&encode(canonical)).is_none());
    }
}
