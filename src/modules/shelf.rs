// SPDX-License-Identifier: GPL-3.0-only

//! The shelf: files dropped on the island, held until dragged out again or shared.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use cosmic::iced::clipboard::mime::{AllowedMimeTypes, AsMimeTypes};

pub const URI_LIST: &str = "text/uri-list";

/// A `text/uri-list` drag payload.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UriList(pub Vec<String>);

impl UriList {
    pub fn parse(text: &str) -> Self {
        Self(
            text.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(str::to_owned)
                .collect(),
        )
    }
}

impl TryFrom<(Vec<u8>, String)> for UriList {
    type Error = std::string::FromUtf8Error;

    fn try_from((data, _mime): (Vec<u8>, String)) -> Result<Self, Self::Error> {
        Ok(Self::parse(&String::from_utf8(data)?))
    }
}

impl AllowedMimeTypes for UriList {
    fn allowed() -> Cow<'static, [String]> {
        Cow::Owned(vec![URI_LIST.to_owned()])
    }
}

impl AsMimeTypes for UriList {
    fn available(&self) -> Cow<'static, [String]> {
        Cow::Owned(vec![URI_LIST.to_owned()])
    }

    fn as_bytes(&self, mime_type: &str) -> Option<Cow<'static, [u8]>> {
        (mime_type == URI_LIST).then(|| Cow::Owned(format!("{}\r\n", self.0.join("\r\n")).into_bytes()))
    }
}

/// A file held on the shelf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub uri: String,
    pub name: String,
    pub path: Option<PathBuf>,
    /// Text pasted or dropped onto the shelf, kept in a snippet file.
    pub text: Option<String>,
}

impl Item {
    pub fn from_uri(uri: String) -> Self {
        let path = uri
            .strip_prefix("file://")
            .map(|p| PathBuf::from(super::media::percent_decode(p)));
        let name = path
            .as_deref()
            .and_then(Path::file_name)
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| uri.clone());
        let text = path
            .as_deref()
            .filter(|p| is_snippet(p) && p.extension().is_some_and(|e| e == "txt"))
            .and_then(|p| std::fs::read_to_string(p).ok());
        Self {
            uri,
            name,
            path,
            text,
        }
    }

    pub fn is_dir(&self) -> bool {
        self.path.as_deref().is_some_and(Path::is_dir)
    }

    /// Whether the file still exists (remote URIs are assumed to).
    pub fn exists(&self) -> bool {
        self.path.as_deref().is_none_or(Path::exists)
    }
}

/// Where text and images pasted onto the shelf are kept as files.
pub fn snippets_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .map(|d| d.join("cosmic-ext-island/snippets"))
}

fn is_snippet(path: &Path) -> bool {
    snippets_dir().is_some_and(|dir| path.starts_with(dir))
}

/// What can be pasted or dropped onto the shelf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Clip {
    Files(UriList),
    Image(Vec<u8>),
    Text(String),
}

const TEXT_MIMES: [&str; 3] = ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"];

impl TryFrom<(Vec<u8>, String)> for Clip {
    type Error = std::string::FromUtf8Error;

    fn try_from((data, mime): (Vec<u8>, String)) -> Result<Self, Self::Error> {
        Ok(match mime.as_str() {
            URI_LIST => Self::Files(UriList::parse(&String::from_utf8(data)?)),
            "image/png" => Self::Image(data),
            _ => {
                let text = String::from_utf8(data)?;
                // Copied files sometimes arrive as plain text URIs.
                let uris = UriList::parse(&text);
                if !uris.0.is_empty() && uris.0.iter().all(|u| u.starts_with("file://")) {
                    Self::Files(uris)
                } else {
                    Self::Text(text)
                }
            }
        })
    }
}

impl AllowedMimeTypes for Clip {
    fn allowed() -> Cow<'static, [String]> {
        // Most specific first: files, then images, then text.
        Cow::Owned(
            [URI_LIST, "image/png"]
                .into_iter()
                .chain(TEXT_MIMES)
                .map(str::to_owned)
                .collect(),
        )
    }
}

/// What a shelf item offers when dragged out: the file, and its text for snippets.
#[derive(Debug, Clone)]
pub struct DragOut {
    pub uris: UriList,
    pub text: Option<String>,
}

impl AsMimeTypes for DragOut {
    fn available(&self) -> Cow<'static, [String]> {
        let mut mimes = vec![URI_LIST.to_owned()];
        if self.text.is_some() {
            mimes.extend(TEXT_MIMES.map(str::to_owned));
        }
        Cow::Owned(mimes)
    }

    fn as_bytes(&self, mime_type: &str) -> Option<Cow<'static, [u8]>> {
        if mime_type == URI_LIST {
            return self.uris.as_bytes(mime_type);
        }
        let text = self.text.as_ref()?;
        TEXT_MIMES
            .contains(&mime_type)
            .then(|| Cow::Owned(text.clone().into_bytes()))
    }
}

/// Saves pasted text or an image as a snippet file; returns its URI.
pub fn save_snippet(clip: &Clip, now: chrono::DateTime<chrono::Local>) -> std::io::Result<String> {
    let dir = snippets_dir().ok_or_else(|| std::io::Error::other("no data directory"))?;
    std::fs::create_dir_all(&dir)?;
    let stamp = now.format("%Y-%m-%d %H-%M-%S");
    let (name, bytes) = match clip {
        Clip::Text(text) => {
            // Name it after its first words, as a reminder of what it says.
            let words: String = text
                .chars()
                .filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(32)
                .collect();
            let words = words.trim();
            let name = if words.is_empty() {
                format!("{stamp}.txt")
            } else {
                format!("{words} ({stamp}).txt")
            };
            (name, text.as_bytes().to_vec())
        }
        Clip::Image(png) => (format!("{stamp}.png"), png.clone()),
        Clip::Files(_) => return Err(std::io::Error::other("files are not snippets")),
    };
    let path = dir.join(name);
    std::fs::write(&path, bytes)?;
    Ok(format!("file://{}", percent_encode_path(&path)))
}

/// Encodes a path for a `file://` URI.
pub fn percent_encode_path(path: &Path) -> String {
    let mut out = String::new();
    for b in path.to_string_lossy().bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Deletes snippet files no shelf holds any more.
pub fn remove_unused_snippets(shelves: &Shelves) {
    let Some(dir) = snippets_dir() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let held: std::collections::HashSet<PathBuf> =
        shelves.all_items().filter_map(|i| i.path.clone()).collect();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && !held.contains(&path) {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Adds URIs that are not on the shelf yet, keeping the order they were dropped in.
pub fn add(items: &mut Vec<Item>, uris: UriList) {
    for uri in uris.0 {
        if !items.iter().any(|i| i.uri == uri) {
            items.push(Item::from_uri(uri));
        }
    }
}

/// A named group of held files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shelf {
    pub name: String,
    pub items: Vec<Item>,
}

/// All shelves and which one is shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shelves {
    pub list: Vec<Shelf>,
    pub current: usize,
}

/// How shelves are stored: names with the URIs they hold.
pub type Stored = Vec<(String, Vec<String>)>;

impl Shelves {
    /// One empty shelf called `name`.
    pub fn new(name: String) -> Self {
        Self {
            list: vec![Shelf {
                name,
                items: Vec::new(),
            }],
            current: 0,
        }
    }

    pub fn from_stored(stored: Stored, current: usize, default_name: String) -> Self {
        let list: Vec<Shelf> = stored
            .into_iter()
            .map(|(name, uris)| {
                // Names saved by 0.1 may hold Unicode isolation marks.
                let name = name.replace(['\u{2068}', '\u{2069}'], "");
                let mut items = Vec::new();
                add(&mut items, UriList(uris));
                // Forget files that were deleted or moved meanwhile.
                items.retain(Item::exists);
                Shelf { name, items }
            })
            .collect();
        if list.is_empty() {
            return Self::new(default_name);
        }
        Self {
            current: current.min(list.len() - 1),
            list,
        }
    }

    pub fn to_stored(&self) -> Stored {
        self.list
            .iter()
            .map(|s| (s.name.clone(), s.items.iter().map(|i| i.uri.clone()).collect()))
            .collect()
    }

    pub fn current(&self) -> &Shelf {
        &self.list[self.current]
    }

    pub fn current_mut(&mut self) -> &mut Shelf {
        &mut self.list[self.current]
    }

    pub fn select(&mut self, index: usize) {
        if index < self.list.len() {
            self.current = index;
        }
    }

    /// Adds dropped files to the front of shelf `index` (the current one when
    /// out of range), where the drop slot opened. Returns the URIs that were new.
    pub fn add_to(&mut self, index: usize, uris: UriList) -> Vec<String> {
        let index = if index < self.list.len() { index } else { self.current };
        let items = &mut self.list[index].items;
        let mut new = Vec::new();
        for uri in uris.0 {
            if !items.iter().any(|i| i.uri == uri) && !new.contains(&uri) {
                new.push(uri);
            }
        }
        let front: Vec<Item> = new.iter().cloned().map(Item::from_uri).collect();
        items.splice(0..0, front);
        new
    }

    /// Appends a new shelf and shows it; returns its index.
    pub fn create(&mut self, name: String) -> usize {
        self.list.push(Shelf {
            name,
            items: Vec::new(),
        });
        self.current = self.list.len() - 1;
        self.current
    }

    pub fn rename(&mut self, index: usize, name: &str) {
        let name = name.trim();
        if let Some(shelf) = self.list.get_mut(index)
            && !name.is_empty()
        {
            shelf.name = name.to_owned();
        }
    }

    /// Deletes shelf `index`, keeping at least one shelf.
    pub fn delete(&mut self, index: usize) {
        if self.list.len() <= 1 || index >= self.list.len() {
            return;
        }
        self.list.remove(index);
        if self.current > index || self.current >= self.list.len() {
            self.current = self.current.saturating_sub(1);
        }
    }

    /// Every item on every shelf.
    pub fn all_items(&self) -> impl Iterator<Item = &Item> {
        self.list.iter().flat_map(|s| s.items.iter())
    }
}

/// Somewhere files from the shelf can be sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShareTarget {
    /// A paired KDE Connect device (also served by Valent and GSConnect); `kind`
    /// is the device type KDE Connect reports: `smartphone`, `tablet`, `desktop`, `laptop` or `tv`.
    KdeConnect { id: String, name: String, kind: String },
}

const KDECONNECT: &str = "org.kde.kdeconnect";

impl ShareTarget {
    pub fn label(&self) -> String {
        match self {
            Self::KdeConnect { name, .. } => name.clone(),
        }
    }

    /// Icon of the kind of device, from the icon theme.
    pub fn icon(&self) -> &'static str {
        let Self::KdeConnect { kind, .. } = self;
        match kind.as_str() {
            "desktop" | "laptop" => "computer-symbolic",
            "tv" => "video-display-symbolic",
            _ => "phone-symbolic",
        }
    }

    /// Sends files to the device, over KDE Connect's D-Bus interface, or with
    /// `kdeconnect-cli` where that is all there is.
    pub async fn share(&self, paths: Vec<PathBuf>) -> Result<(), String> {
        let Self::KdeConnect { id, .. } = self;
        if kdeconnect_running().await {
            let connection = super::media::connection().await.map_err(|e| e.to_string())?;
            let urls: Vec<String> = paths
                .iter()
                .map(|p| format!("file://{}", p.display()))
                .collect();
            connection
                .call_method(
                    Some(KDECONNECT),
                    format!("/modules/kdeconnect/devices/{id}/share").as_str(),
                    Some("org.kde.kdeconnect.device.share"),
                    "shareUrls",
                    &(urls,),
                )
                .await
                .map_err(|e| e.to_string())?;
            return Ok(());
        }
        for path in paths {
            tokio::process::Command::new("kdeconnect-cli")
                .args(["--device", id, "--share"])
                .arg(path)
                .status()
                .await
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

/// Whether the KDE Connect daemon runs; asking it something would start it.
async fn kdeconnect_running() -> bool {
    let Ok(connection) = super::media::connection().await else {
        return false;
    };
    let Ok(dbus) = zbus::fdo::DBusProxy::new(connection).await else {
        return false;
    };
    let Ok(name) = zbus::names::BusName::try_from(KDECONNECT) else {
        return false;
    };
    dbus.name_has_owner(name).await.unwrap_or(false)
}

fn in_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|p| p.join(program).is_file()))
}

/// Finds paired KDE Connect devices that are in reach.
/// (LocalSend devices are found by [`super::localsend`].)
pub async fn detect_share_targets() -> Vec<ShareTarget> {
    if kdeconnect_running().await {
        let devices = async {
            let connection = super::media::connection().await.ok()?;
            let reply = connection
                .call_method(
                    Some(KDECONNECT),
                    "/modules/kdeconnect",
                    Some("org.kde.kdeconnect.daemon"),
                    "deviceNames",
                    &(true, true),
                )
                .await
                .ok()?;
            reply.body().deserialize::<std::collections::HashMap<String, String>>().ok()
        }
        .await;
        let mut targets = devices.map(kdeconnect_devices).unwrap_or_default();
        for target in &mut targets {
            let ShareTarget::KdeConnect { id, kind, .. } = target;
            if let Some(found) = kdeconnect_kind(id).await {
                *kind = found;
            }
        }
        return targets;
    }
    if !in_path("kdeconnect-cli") {
        return Vec::new();
    }
    match tokio::process::Command::new("kdeconnect-cli")
        .args(["--list-available", "--id-name-only"])
        .output()
        .await
    {
        Ok(output) => parse_kdeconnect_devices(&String::from_utf8_lossy(&output.stdout)).collect(),
        Err(_) => Vec::new(),
    }
}

/// The type KDE Connect reports for a device.
async fn kdeconnect_kind(id: &str) -> Option<String> {
    let connection = super::media::connection().await.ok()?;
    let reply = connection
        .call_method(
            Some(KDECONNECT),
            format!("/modules/kdeconnect/devices/{id}").as_str(),
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.kde.kdeconnect.device", "type"),
        )
        .await
        .ok()?;
    let value: zbus::zvariant::OwnedValue = reply.body().deserialize().ok()?;
    String::try_from(value).ok()
}

/// Devices by id, as KDE Connect's daemon lists them, sorted by name.
fn kdeconnect_devices(devices: std::collections::HashMap<String, String>) -> Vec<ShareTarget> {
    let mut targets: Vec<_> = devices
        .into_iter()
        .map(|(id, name)| ShareTarget::KdeConnect {
            id,
            name,
            kind: String::new(),
        })
        .collect();
    targets.sort_by_key(ShareTarget::label);
    targets
}

/// Parses `kdeconnect-cli --id-name-only` output: one `id name` per line.
fn parse_kdeconnect_devices(output: &str) -> impl Iterator<Item = ShareTarget> + '_ {
    output.lines().filter_map(|line| {
        let (id, name) = line.trim().split_once(' ')?;
        Some(ShareTarget::KdeConnect {
            id: id.to_owned(),
            name: name.trim().to_owned(),
            kind: String::new(),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_uri_lists() {
        let list = UriList::parse("# comment\r\nfile:///home/j/a%20b.txt\r\n\r\nfile:///tmp/c\n");
        assert_eq!(list.0, ["file:///home/j/a%20b.txt", "file:///tmp/c"]);
    }

    #[test]
    fn serializes_uri_lists() {
        let list = UriList(vec!["file:///a".into(), "file:///b".into()]);
        assert_eq!(
            list.as_bytes(URI_LIST).unwrap().as_ref(),
            b"file:///a\r\nfile:///b\r\n"
        );
        assert!(list.as_bytes("text/plain").is_none());
    }

    #[test]
    fn items_decode_names_and_skip_duplicates() {
        let mut items = Vec::new();
        add(
            &mut items,
            UriList(vec!["file:///home/j/My%20File.pdf".into(), "file:///home/j/My%20File.pdf".into()]),
        );
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "My File.pdf");
        assert_eq!(items[0].path.as_deref(), Some(Path::new("/home/j/My File.pdf")));
    }

    fn uris(names: &[&str]) -> UriList {
        UriList(names.iter().map(|n| format!("http://x/{n}")).collect())
    }

    #[test]
    fn shelves_round_trip() {
        let mut shelves = Shelves::new("Files".into());
        shelves.add_to(0, uris(&["a"]));
        shelves.create("Work".into());
        shelves.add_to(1, uris(&["b", "c"]));
        assert_eq!(shelves.add_to(1, uris(&["d", "b"])), ["http://x/d"]);
        assert_eq!(shelves.current().items[0].uri, "http://x/d");
        let stored = shelves.to_stored();
        let back = Shelves::from_stored(stored, 1, "Files".into());
        assert_eq!(back, shelves);
        assert_eq!(back.current().name, "Work");
    }

    #[test]
    fn last_shelf_cannot_be_deleted() {
        let mut shelves = Shelves::new("Files".into());
        shelves.delete(0);
        assert_eq!(shelves.list.len(), 1);
    }

    #[test]
    fn deleting_keeps_a_valid_selection() {
        let mut shelves = Shelves::new("A".into());
        shelves.create("B".into());
        shelves.create("C".into());
        assert_eq!(shelves.current, 2);
        shelves.delete(2);
        assert_eq!(shelves.current().name, "B");
        shelves.select(0);
        shelves.delete(1);
        assert_eq!(shelves.current().name, "A");
    }

    #[test]
    fn empty_names_are_ignored() {
        let mut shelves = Shelves::new("A".into());
        shelves.rename(0, "   ");
        assert_eq!(shelves.current().name, "A");
        shelves.rename(0, " Práce ");
        assert_eq!(shelves.current().name, "Práce");
    }

    #[test]
    fn nothing_stored_gives_one_shelf() {
        let shelves = Shelves::from_stored(Vec::new(), 3, "Files".into());
        assert_eq!(shelves.list.len(), 1);
        assert_eq!(shelves.current, 0);
    }

    #[test]
    fn clips_by_mime_type() {
        let files = Clip::try_from((b"file:///a\r\n".to_vec(), URI_LIST.to_owned())).unwrap();
        assert_eq!(files, Clip::Files(UriList(vec!["file:///a".into()])));
        let text = Clip::try_from((b"ahoj".to_vec(), "text/plain".to_owned())).unwrap();
        assert_eq!(text, Clip::Text("ahoj".into()));
        // File managers put copied files on the clipboard as text too.
        let copied = Clip::try_from((b"file:///b".to_vec(), "UTF8_STRING".to_owned())).unwrap();
        assert!(matches!(copied, Clip::Files(_)));
    }

    #[test]
    fn dragging_a_snippet_offers_its_text() {
        let drag = DragOut {
            uris: UriList(vec!["file:///x.txt".into()]),
            text: Some("ahoj".into()),
        };
        assert!(drag.available().iter().any(|m| m == "text/plain"));
        assert_eq!(drag.as_bytes("text/plain").unwrap().as_ref(), b"ahoj");
        let file = DragOut {
            uris: UriList(vec!["file:///x.pdf".into()]),
            text: None,
        };
        assert_eq!(file.available().as_ref(), [URI_LIST.to_owned()]);
    }

    #[test]
    fn snippets_are_named_after_their_text() {
        let dir = std::env::temp_dir().join(format!("island-snip-{}", std::process::id()));
        unsafe { std::env::set_var("XDG_DATA_HOME", &dir) };
        let now = chrono::Local::now();
        let uri = save_snippet(&Clip::Text("Nákup:  mléko / chleba\n".into()), now).unwrap();
        let item = Item::from_uri(uri);
        assert!(item.name.starts_with("Nákup mléko chleba ("), "{}", item.name);
        assert_eq!(item.text.as_deref(), Some("Nákup:  mléko / chleba\n"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn kdeconnect_devices_by_name() {
        let devices = kdeconnect_devices(
            [("b1".to_string(), "Pixel".to_string()), ("a2".to_string(), "Laptop".to_string())]
                .into(),
        );
        assert_eq!(devices.iter().map(ShareTarget::label).collect::<Vec<_>>(), ["Laptop", "Pixel"]);
    }

    #[test]
    fn parses_kdeconnect_devices() {
        let devices: Vec<_> = parse_kdeconnect_devices("abc123 Pixel 8\nxyz Laptop\n").collect();
        assert_eq!(
            devices[0],
            ShareTarget::KdeConnect {
                id: "abc123".into(),
                name: "Pixel 8".into(),
                kind: String::new(),
            }
        );
        assert_eq!(devices.len(), 2);
    }
}
