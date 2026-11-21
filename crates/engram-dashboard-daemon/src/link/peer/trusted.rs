//! The trusted list and network settings — `peers.<account>.<machine>.json` in the daemon state folder — and its file rules.
//!
//! The file is `{version, mac, body}`: `body` = the list serialized as a JSON string, `mac` = its MAC under this account's identity key
//!   ([`super::mac`]) as 64 hex digits. ★The MAC is over the exact stored bytes of `body`★ — the body is never re-serialized before it is
//!   checked. The file follows base `file`'s rules (version key, cap, a newer version or a failed read = refuse to write, corrupt = set
//!   aside on the first write, the overwrite decision made again right before every write); a body that does not parse or breaks an
//!   entry rule is corrupt. Whether the MAC matches is not a file rule — the state decides what a mismatch means.
//!
//! Entry rules ([`check_entries`]): IDs unique; names required, unique ignoring case, no `@`, no whitespace or control characters;
//!   `permission` = `mail` only; a direct address has a non-zero port. Our own ID is refused by the state on add (it is not known here).

// ADR-0291, ADR-0389

use std::collections::HashSet;
use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::path::Path;

use engram_dashboard_base::file::{self, Claim, Loaded, Parsed, Refused, Spec, WritePolicy};
use engram_dashboard_base::text::hex_lower;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::id::{decode_hex32, PeerId};
use crate::file_hooks::OS_HOOKS;

/// Default UDP listen port while `network.port` is absent — release builds.
pub(crate) const RELEASE_DEFAULT_PORT: u16 = 47_120;
/// Default UDP listen port for dev builds (data root `.engram-dev`) — differs from release so both daemons run on one PC.
pub(crate) const DEV_DEFAULT_PORT: u16 = 47_121;
/// Longest PC name, in bytes (the remote agent-name bound).
pub(crate) const MAX_NAME_BYTES: usize = 256;
/// Longest helper-server URL, in bytes.
const MAX_URL_BYTES: usize = 2048;

const FILE_VERSION: u64 = 1;
const VERSION_KEY: &str = "version";
const MAC_KEY: &str = "mac";
const BODY_KEY: &str = "body";

// ADR-0291 R1–R7: version key · cap · shape check inside `parse`.
pub(crate) const SPEC: Spec = Spec {
    version_key: VERSION_KEY,
    current: FILE_VERSION,
    cap: 1 << 20,
    shape,
};

/// The list as stored in `body`. A later group record goes beside `peers` as another top-level key.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TrustedList {
    #[serde(default)]
    pub(crate) peers: Vec<TrustedPeer>,
    #[serde(default)]
    pub(crate) network: NetworkSettings,
}

/// One trusted PC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TrustedPeer {
    pub(crate) id: PeerId,
    /// My label for that PC — the `name@pc` suffix.
    pub(crate) name: String,
    /// A direct `ip:port` to try besides relay and lookup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) address: Option<SocketAddr>,
    pub(crate) permission: Permission,
}

/// What a trusted PC may do here. Only `mail` exists in this version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Permission {
    Mail,
}

/// How this PC reaches others.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct NetworkSettings {
    /// UDP listen port; absent = [`NetworkSettings::listen_port`]'s default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) port: Option<u16>,
    #[serde(default)]
    pub(crate) relay: Helper,
    #[serde(default)]
    pub(crate) address_lookup: Helper,
    #[serde(default = "mdns_default")]
    pub(crate) mdns: bool,
}

fn mdns_default() -> bool {
    true
}

impl Default for NetworkSettings {
    fn default() -> Self {
        Self {
            port: None,
            relay: Helper::Default,
            address_lookup: Helper::Default,
            mdns: true,
        }
    }
}

impl NetworkSettings {
    /// The port to listen on — `port` if set, else the build's default (`dev` = the `.engram-dev` data root).
    pub(crate) fn listen_port(&self, dev: bool) -> u16 {
        self.port.unwrap_or(if dev {
            DEV_DEFAULT_PORT
        } else {
            RELEASE_DEFAULT_PORT
        })
    }

    /// The form rules a stored file is read by (the URLs' own form is [`Helper::parse`]'s, at deserialization).
    fn check(&self) -> Result<(), String> {
        if self.port == Some(0) {
            return Err("the listen port must not be 0".to_string());
        }
        Ok(())
    }

    /// Each helper URL through the link adapter's own parse for its role (transport `iroh::check_relay_url` / `check_lookup_url`).
    ///   ★Run on writes, not on load★: a later adapter or `url` upgrade that parses more strictly must not turn a stored list corrupt
    ///   (peers dropped, settings reset) — the store calls it when the settings change, and the endpoint build is where a stored setting
    ///   the adapter refuses shows up (an endpoint error).
    pub(crate) fn check_for_endpoint(&self) -> Result<(), String> {
        if let Helper::Url(url) = &self.relay {
            engram_dashboard_transport::iroh::check_relay_url(url)?;
        }
        if let Helper::Url(url) = &self.address_lookup {
            engram_dashboard_transport::iroh::check_lookup_url(url)?;
        }
        Ok(())
    }
}

/// A helper server choice (relay or address lookup), stored as `"default"` | `"none"` | a URL.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) enum Helper {
    /// n0's public servers.
    #[default]
    Default,
    /// Nothing is contacted for this role.
    None,
    /// The user's own server.
    Url(String),
}

impl Helper {
    /// `"default"`, `"none"`, or an `http://` / `https://` URL with a real host ([`authority_ok`]), no whitespace or control
    ///   characters, at most 2048 bytes.
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        match text {
            "default" => return Ok(Self::Default),
            "none" => return Ok(Self::None),
            _ => {}
        }
        let rest = text
            .strip_prefix("https://")
            .or_else(|| text.strip_prefix("http://"));
        let authority = rest.map(|rest| rest.split(['/', '?', '#']).next().unwrap_or(""));
        if text.len() > MAX_URL_BYTES
            || !authority.is_some_and(authority_ok)
            || text.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return Err("a helper server is \"default\", \"none\" or an http(s) URL".to_string());
        }
        Ok(Self::Url(text.to_string()))
    }
}

/// A URL authority this module takes: a host — a DNS name (dot-separated labels of ASCII letters, digits and `-`, no label empty or
///   starting or ending with `-`, the last label not a number), an IPv4 address in plain dotted four-part form, or an IPv6 address in
///   brackets — and an optional `:port` from 1 to 65535. No user info.
fn authority_ok(authority: &str) -> bool {
    let (host_ok, port) = if let Some(inner) = authority.strip_prefix('[') {
        match inner.split_once(']') {
            Some((v6, rest)) => (v6.parse::<std::net::Ipv6Addr>().is_ok(), rest),
            None => return false,
        }
    } else {
        let (host, port) = match authority.find(':') {
            Some(i) => (&authority[..i], &authority[i..]),
            None => (authority, ""),
        };
        let labels_ok = !host.is_empty()
            && host.split('.').all(|label| {
                !label.is_empty()
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            });
        // A host whose last label is a number (decimal, or `0x` hex) is read as an IPv4 address by URL parsers, shorthand included
        //   (`0` → 0.0.0.0, `1.2.3` → 1.2.0.3); only the plain dotted four-part form is taken, so the host stored is the host used.
        let last = host.rsplit('.').next().unwrap_or("");
        let numeric_last = !last.is_empty()
            && (last.bytes().all(|b| b.is_ascii_digit())
                || last.len() >= 2
                    && last[..2].eq_ignore_ascii_case("0x")
                    && last[2..].bytes().all(|b| b.is_ascii_hexdigit()));
        let host_ok = if numeric_last {
            host.parse::<std::net::Ipv4Addr>().is_ok()
        } else {
            labels_ok
        };
        (host_ok, port)
    };
    let port_ok = match port {
        "" => true,
        p => p
            .strip_prefix(':')
            .filter(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|d| d.parse::<u16>().ok())
            .is_some_and(|n| n != 0),
    };
    host_ok && port_ok
}

impl TryFrom<String> for Helper {
    type Error = String;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Helper::parse(&text)
    }
}

impl From<Helper> for String {
    fn from(helper: Helper) -> Self {
        match helper {
            Helper::Default => "default".to_string(),
            Helper::None => "none".to_string(),
            Helper::Url(url) => url,
        }
    }
}

/// A PC name: required, at most [`MAX_NAME_BYTES`], no `@`, no whitespace or control characters.
pub(crate) fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("a PC name is required".to_string());
    }
    if name.len() > MAX_NAME_BYTES {
        return Err(format!("a PC name is at most {MAX_NAME_BYTES} bytes"));
    }
    if name.contains('@')
        || name
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || is_invisible(c))
    {
        return Err(
            "a PC name has no '@', spaces, control or invisible formatting characters".to_string(),
        );
    }
    Ok(())
}

/// Invisible characters: Unicode general category Cf (format — zero-width, bidi controls, soft hyphen, tags …) together with the
///   Default_Ignorable_Code_Point property (variation selectors, Hangul fillers, Mongolian free variation selectors …). Two names that
///   look the same could otherwise pass the uniqueness check. Ranges merged from Unicode 16.0 (`DerivedCoreProperties.txt` and Python
///   `unicodedata` 16.0).
fn is_invisible(c: char) -> bool {
    const INVISIBLE: &[(u32, u32)] = &[
        (0x00AD, 0x00AD),
        (0x034F, 0x034F),
        (0x0600, 0x0605),
        (0x061C, 0x061C),
        (0x06DD, 0x06DD),
        (0x070F, 0x070F),
        (0x0890, 0x0891),
        (0x08E2, 0x08E2),
        (0x115F, 0x1160),
        (0x17B4, 0x17B5),
        (0x180B, 0x180F),
        (0x200B, 0x200F),
        (0x202A, 0x202E),
        (0x2060, 0x206F),
        (0x3164, 0x3164),
        (0xFE00, 0xFE0F),
        (0xFEFF, 0xFEFF),
        (0xFFA0, 0xFFA0),
        (0xFFF0, 0xFFFB),
        (0x110BD, 0x110BD),
        (0x110CD, 0x110CD),
        (0x13430, 0x1343F),
        (0x1BCA0, 0x1BCA3),
        (0x1D173, 0x1D17A),
        (0xE0000, 0xE0FFF),
    ];
    let c = c as u32;
    INVISIBLE.iter().any(|&(lo, hi)| (lo..=hi).contains(&c))
}

/// The entry rules over the whole list (module head).
pub(crate) fn check_entries(list: &TrustedList) -> Result<(), String> {
    let mut ids = HashSet::new();
    let mut names = HashSet::new();
    for peer in &list.peers {
        check_name(&peer.name)?;
        if !ids.insert(peer.id) {
            return Err(format!("the ID {} is listed twice", peer.id));
        }
        if !names.insert(peer.name.to_lowercase()) {
            return Err(format!("the name \"{}\" is listed twice", peer.name));
        }
        if peer.address.is_some_and(|a| a.port() == 0) {
            return Err(format!("the address of \"{}\" has port 0", peer.name));
        }
    }
    list.network.check()
}

/// A list file as found on disk.
#[derive(Debug)]
pub(crate) enum ListFile {
    Missing,
    /// Readable and well-formed; the MAC is not checked here.
    Signed {
        list: TrustedList,
        body: String,
        mac: [u8; 32],
    },
    /// Unusable content (base `file`'s corrupt) — the reason.
    Corrupt(String),
    /// Must not be written over (newer version, failed read).
    Refused(ListRefusal),
}

impl ListFile {
    /// Does the file list at least one PC (a corrupt or refused file is not known to).
    pub(crate) fn names_peers(&self) -> bool {
        matches!(self, Self::Signed { list, .. } if !list.peers.is_empty())
    }
}

/// Why the list file must not be written over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ListRefusal {
    /// Written by a newer Engram.
    Newer { found: u64 },
    /// The read failed — what it holds is unknown.
    ReadFailed(String),
}

impl fmt::Display for ListRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Newer { found } => write!(
                f,
                "the trusted list file was written by a newer Engram (version {found}) and is left as it is"
            ),
            Self::ReadFailed(e) => write!(f, "the trusted list file could not be read: {e}"),
        }
    }
}

/// Reads the list file at `path`.
pub(crate) fn read_list(path: &Path) -> ListFile {
    match file::load(path, &SPEC, OS_HOOKS) {
        Loaded::Missing => ListFile::Missing,
        Loaded::Failed(e) => ListFile::Refused(ListRefusal::ReadFailed(e.to_string())),
        Loaded::Parsed(Parsed::Newer { found }) => ListFile::Refused(ListRefusal::Newer { found }),
        Loaded::Parsed(Parsed::Unusable(reason)) => ListFile::Corrupt(reason),
        // `shape` already passed this document, so these reads cannot fail; a failure is still reported as corrupt.
        Loaded::Parsed(Parsed::Usable { doc, .. }) => match signed_parts(&doc) {
            Ok((list, body, mac)) => ListFile::Signed { list, body, mac },
            Err(reason) => ListFile::Corrupt(reason),
        },
    }
}

// ADR-0291 R7: a body that does not parse or breaks an entry rule makes the whole file corrupt.
fn shape(_version: u64, doc: &Map<String, Value>) -> Result<(), String> {
    signed_parts(doc).map(drop)
}

fn signed_parts(doc: &Map<String, Value>) -> Result<(TrustedList, String, [u8; 32]), String> {
    let mac = doc
        .get(MAC_KEY)
        .and_then(Value::as_str)
        .and_then(decode_hex32)
        .ok_or("`mac` is not 64 hex digits")?;
    let body = doc
        .get(BODY_KEY)
        .and_then(Value::as_str)
        .ok_or("`body` is not a string")?;
    let list: TrustedList =
        serde_json::from_str(body).map_err(|e| format!("`body` is not a trusted list: {e}"))?;
    check_entries(&list)?;
    Ok((list, body.to_string(), mac))
}

/// The body text of `list` — what the MAC is computed over.
pub(crate) fn body_of(list: &TrustedList) -> Result<String, String> {
    serde_json::to_string(list)
        .map_err(|e| format!("the trusted list could not be serialized: {e}"))
}

/// The whole file text for a body and its MAC.
pub(crate) fn file_text(body: &str, mac: &[u8; 32]) -> Result<String, String> {
    let mut doc = Map::new();
    doc.insert(VERSION_KEY.to_string(), Value::from(FILE_VERSION));
    doc.insert(MAC_KEY.to_string(), Value::from(hex_lower(mac)));
    doc.insert(BODY_KEY.to_string(), Value::from(body));
    serde_json::to_string_pretty(&Value::Object(doc))
        .map_err(|e| format!("the trusted list file could not be serialized: {e}"))
}

/// Why a list write did not happen.
#[derive(Debug)]
pub(crate) enum WriteError {
    /// The file must not be written over now (newer version, failed read) — the same answer until it changes.
    Refused(ListRefusal),
    /// The text is over the cap.
    TooLarge(String),
    /// The write or the set-aside failed.
    Io(io::Error),
}

/// Writes `text` to `path` under base `file`'s rules: the current file is read again and decided right before the write
///   ([`Loaded::write_policy`] with `claim`); a usable file this run has not claimed and a corrupt file are set aside first (a failed
///   set-aside fails the write, the original untouched).
// ADR-0291 R6 · R8 · R16
pub(crate) fn write_list(path: &Path, text: &str, claim: Claim) -> Result<(), WriteError> {
    file::check_cap(text.len(), &SPEC).map_err(|e| WriteError::TooLarge(e.to_string()))?;
    let current = file::load(path, &SPEC, OS_HOOKS);
    match current.write_policy(claim) {
        WritePolicy::Write => {}
        WritePolicy::CopyAsideFirst => {
            file::copy_aside(path, OS_HOOKS).map_err(WriteError::Io)?;
        }
        WritePolicy::Refuse(Refused::Newer { found }) => {
            return Err(WriteError::Refused(ListRefusal::Newer { found }))
        }
        WritePolicy::Refuse(Refused::ReadFailed) => {
            let why = match current {
                Loaded::Failed(e) => e.to_string(),
                _ => "unknown".to_string(),
            };
            return Err(WriteError::Refused(ListRefusal::ReadFailed(why)));
        }
    }
    file::write_atomic(path, text.as_bytes(), OS_HOOKS).map_err(WriteError::Io)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> PeerId {
        PeerId::from_bytes([n; 32])
    }

    fn peer(n: u8, name: &str) -> TrustedPeer {
        TrustedPeer {
            id: id(n),
            name: name.to_string(),
            address: None,
            permission: Permission::Mail,
        }
    }

    #[test]
    fn names_follow_the_rules() {
        assert!(check_name("office-pc").is_ok());
        assert!(check_name(&"n".repeat(MAX_NAME_BYTES)).is_ok());
        assert!(
            check_name("사무실-pc").is_ok(),
            "letters of any script are fine"
        );
        for bad in [
            String::new(),
            "a@b".to_string(),
            "a b".to_string(),
            "a\tb".to_string(),
            "a\u{7}b".to_string(),
            "n".repeat(MAX_NAME_BYTES + 1),
            "home\u{200B}".to_string(),
            "\u{202E}emoh".to_string(),
            "ho\u{00AD}me".to_string(),
            "home\u{FEFF}".to_string(),
            "home\u{E0041}".to_string(),
            "home\u{FE0F}".to_string(),
            "home\u{E0100}".to_string(),
            "ho\u{034F}me".to_string(),
            "\u{3164}".to_string(),
            "home\u{1160}".to_string(),
            "ho\u{FFA0}me".to_string(),
            "home\u{180B}".to_string(),
        ] {
            assert!(check_name(&bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn ids_and_names_are_unique_and_names_ignore_case() {
        let mut list = TrustedList {
            peers: vec![peer(1, "home"), peer(2, "office")],
            ..TrustedList::default()
        };
        assert!(check_entries(&list).is_ok());
        list.peers.push(peer(3, "HOME"));
        assert!(check_entries(&list).is_err());
        list.peers.pop();
        list.peers.push(peer(1, "laptop"));
        assert!(check_entries(&list).is_err());
    }

    #[test]
    fn helpers_are_default_none_or_an_http_url() {
        assert_eq!(Helper::parse("default"), Ok(Helper::Default));
        assert_eq!(Helper::parse("none"), Ok(Helper::None));
        assert_eq!(
            Helper::parse("https://relay.example.net"),
            Ok(Helper::Url("https://relay.example.net".to_string()))
        );
        for good in [
            "https://xn--a.example",
            "https://relay.example.net:443/path?x=1#f",
            "http://10.0.0.1",
            "http://[::1]:8080/",
            "https://localhost",
        ] {
            assert_eq!(
                Helper::parse(good),
                Ok(Helper::Url(good.to_string())),
                "{good}"
            );
        }
        for bad in [
            "",
            "Default",
            "relay.example.net",
            "ftp://x",
            "https://",
            "https:///path",
            "https://?x",
            "https://#a",
            "https://:80",
            "https://user@host",
            "https://host:0",
            "https://host:99999",
            "https://host:",
            "https://a..b",
            "https://-a.b",
            "http://[::1",
            "http://[nope]",
            "https://999.999.999.999",
            "https://0",
            "https://1.2.3",
            "https://0x7f.1",
            "https://0x",
            "https://1.2.3.0x",
            "https://a.0X",
            "https://example.123",
            "https://a b",
            "https://a\nb",
        ] {
            assert!(Helper::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_listen_port_defaults_differ_between_dev_and_release() {
        let mut network = NetworkSettings::default();
        assert_eq!(network.listen_port(false), RELEASE_DEFAULT_PORT);
        assert_eq!(network.listen_port(true), DEV_DEFAULT_PORT);
        assert_ne!(RELEASE_DEFAULT_PORT, DEV_DEFAULT_PORT);
        network.port = Some(5000);
        assert_eq!(network.listen_port(true), 5000);
        network.port = Some(0);
        assert!(network.check().is_err());
        network.port = None;
        network.relay = Helper::Url("https://relay.example.net".to_string());
        network.address_lookup = Helper::Url("https://dns.example.net/pkarr".to_string());
        assert!(network.check_for_endpoint().is_ok());
        // Built around the parser (as a hand-edited file could be): the adapter's own parse still refuses it — but only the endpoint
        //   check, never the read rule.
        network.relay = Helper::Url("https://999.999.999.999".to_string());
        assert!(network.check_for_endpoint().is_err());
        assert!(network.check().is_ok());
    }

    #[test]
    fn the_body_round_trips_with_defaults_filled_in() {
        let list = TrustedList {
            peers: vec![TrustedPeer {
                address: Some("192.168.0.7:47120".parse().unwrap()),
                ..peer(1, "home")
            }],
            network: NetworkSettings {
                port: Some(5000),
                relay: Helper::None,
                address_lookup: Helper::Url("https://dns.example.net".to_string()),
                mdns: false,
            },
        };
        let body = body_of(&list).unwrap();
        assert_eq!(serde_json::from_str::<TrustedList>(&body).unwrap(), list);
        let bare: TrustedList = serde_json::from_str("{}").unwrap();
        assert_eq!(bare, TrustedList::default());
        assert!(bare.network.mdns);
    }

    #[test]
    fn a_permission_other_than_mail_is_not_a_list() {
        let body = format!(
            r#"{{"peers":[{{"id":"{}","name":"a","permission":"control"}}]}}"#,
            id(1)
        );
        assert!(serde_json::from_str::<TrustedList>(&body).is_err());
    }
}
