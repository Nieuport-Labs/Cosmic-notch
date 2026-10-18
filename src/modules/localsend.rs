// SPDX-License-Identifier: GPL-3.0-only

//! Sending files to nearby devices over the LocalSend protocol (v2), without
//! the LocalSend app on this computer.
//!
//! Discovery: devices announce themselves by UDP multicast; we announce too
//! and take replies on a small HTTP endpoint, and also probe the local subnet
//! for devices on networks that drop multicast. Sending: `prepare-upload`
//! (which waits until the other side accepts), then one `upload` per file.
//! See <https://github.com/localsend/protocol>.

use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use cosmic::iced::futures::{SinkExt, Stream, channel::mpsc};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const MULTICAST: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 167);
const PORT: u16 = 53317;
const API: &str = "/api/localsend/v2";

/// What a device says about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub alias: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub device_model: Option<String>,
    #[serde(default)]
    pub device_type: Option<String>,
    pub fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub announce: Option<bool>,
}

/// A device found on the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub info: DeviceInfo,
    pub ip: IpAddr,
    pub port: u16,
    pub https: bool,
}

impl Device {
    fn url(&self, path: &str) -> String {
        let scheme = if self.https { "https" } else { "http" };
        format!("{scheme}://{}:{}{API}{path}", self.ip, self.port)
    }

    /// Symbolic icon for the kind of device.
    pub fn icon(&self) -> &'static str {
        match self.info.device_type.as_deref() {
            Some("mobile") => "phone-symbolic",
            Some("web") => "web-browser-symbolic",
            Some("server" | "headless") => "network-server-symbolic",
            _ => "computer-symbolic",
        }
    }
}

/// How this computer introduces itself.
fn own_info(port: Option<u16>) -> DeviceInfo {
    static FINGERPRINT: OnceLock<String> = OnceLock::new();
    let fingerprint = FINGERPRINT.get_or_init(|| {
        let mut bytes = [0u8; 16];
        if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
            use std::io::Read;
            let _ = f.read_exact(&mut bytes);
        }
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    });
    let alias = std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|h| h.trim().to_owned())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "COSMIC".to_owned());
    DeviceInfo {
        alias,
        version: "2.1".to_owned(),
        device_model: Some("Linux".to_owned()),
        device_type: Some("desktop".to_owned()),
        fingerprint: fingerprint.clone(),
        port,
        protocol: port.map(|_| "http".to_owned()),
        download: Some(false),
        announce: None,
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        // LocalSend devices use self-signed certificates.
        .danger_accept_invalid_certs(true)
        .connect_timeout(Duration::from_secs(3))
        .build()
        .expect("TLS backend available")
}

/// Finds devices until the stream is dropped; each device is yielded once.
pub fn discover() -> impl Stream<Item = Device> {
    cosmic::iced::stream::channel(32, |output: mpsc::Sender<Device>| async move {
        let (found_tx, mut found_rx) = tokio::sync::mpsc::unbounded_channel::<Device>();

        let http = tokio::net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).await.ok();
        let http_port = http.as_ref().and_then(|l| l.local_addr().ok()).map(|a| a.port());
        if let Some(listener) = http {
            tokio::spawn(serve_register(listener, found_tx.clone()));
        }
        match multicast_socket() {
            Ok(socket) => {
                tokio::spawn(multicast(socket, http_port, found_tx.clone()));
            }
            Err(err) => tracing::warn!("LocalSend multicast unavailable: {err}"),
        }
        tokio::spawn(scan_subnet(found_tx));

        let own = own_info(None).fingerprint;
        let mut seen = HashSet::new();
        let mut output = output;
        while let Some(device) = found_rx.recv().await {
            if device.info.fingerprint == own || !seen.insert(device.info.fingerprint.clone()) {
                continue;
            }
            if output.send(device).await.is_err() {
                break;
            }
        }
        std::future::pending().await
    })
}

/// Joins the LocalSend multicast group, sharing the port with a running LocalSend app.
fn multicast_socket() -> std::io::Result<tokio::net::UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    socket.set_reuse_port(true)?;
    socket.bind(&SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, PORT)).into())?;
    socket.join_multicast_v4(&MULTICAST, &Ipv4Addr::UNSPECIFIED)?;
    socket.set_nonblocking(true)?;
    tokio::net::UdpSocket::from_std(socket.into())
}

async fn multicast(
    socket: tokio::net::UdpSocket,
    http_port: Option<u16>,
    found: tokio::sync::mpsc::UnboundedSender<Device>,
) {
    let socket = Arc::new(socket);
    let announcement = {
        let mut info = own_info(http_port);
        info.announce = Some(true);
        serde_json::to_vec(&info).unwrap_or_default()
    };
    let announcer = {
        let socket = socket.clone();
        tokio::spawn(async move {
            loop {
                let _ = socket.send_to(&announcement, (MULTICAST, PORT)).await;
                tokio::time::sleep(Duration::from_secs(4)).await;
            }
        })
    };

    let mut buf = vec![0u8; 8192];
    loop {
        let Ok((len, from)) = socket.recv_from(&mut buf).await else {
            break;
        };
        let Ok(info) = serde_json::from_slice::<DeviceInfo>(&buf[..len]) else {
            continue;
        };
        let device = Device {
            port: info.port.unwrap_or(PORT),
            https: info.protocol.as_deref() != Some("http"),
            ip: from.ip(),
            info,
        };
        if found.send(device).is_err() {
            break;
        }
    }
    announcer.abort();
}

/// Answers devices that reply to our announcement over HTTP.
async fn serve_register(
    listener: tokio::net::TcpListener,
    found: tokio::sync::mpsc::UnboundedSender<Device>,
) {
    while let Ok((mut stream, peer)) = listener.accept().await {
        let found = found.clone();
        tokio::spawn(async move {
            let Some(body) = read_http_body(&mut stream).await else {
                return;
            };
            if let Ok(info) = serde_json::from_slice::<DeviceInfo>(&body) {
                let _ = found.send(Device {
                    port: info.port.unwrap_or(PORT),
                    https: info.protocol.as_deref() != Some("http"),
                    ip: peer.ip(),
                    info,
                });
            }
            let reply = serde_json::to_vec(&own_info(None)).unwrap_or_default();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                reply.len()
            );
            let _ = stream.write_all(head.as_bytes()).await;
            let _ = stream.write_all(&reply).await;
        });
    }
}

/// Reads one small HTTP request and returns its body.
async fn read_http_body(stream: &mut tokio::net::TcpStream) -> Option<Vec<u8>> {
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    let read = async {
        loop {
            let n = stream.read(&mut buf).await.ok()?;
            if n == 0 {
                return None;
            }
            data.extend_from_slice(&buf[..n]);
            if let Some(end) = find(&data, b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&data[..end]).to_ascii_lowercase();
                let length = head
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .and_then(|v| v.trim().parse::<usize>().ok())
                    .unwrap_or(0)
                    .min(64 * 1024);
                while data.len() < end + 4 + length {
                    let n = stream.read(&mut buf).await.ok()?;
                    if n == 0 {
                        break;
                    }
                    data.extend_from_slice(&buf[..n]);
                }
                return Some(data[end + 4..].to_vec());
            }
            if data.len() > 64 * 1024 {
                return None;
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(5), read).await.ok().flatten()
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// This computer's address on the local network.
fn local_ipv4() -> Option<Ipv4Addr> {
    // Connecting a UDP socket sends nothing; it only picks the outgoing interface.
    let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(192, 168, 0, 1), 9)).ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if !ip.is_loopback() => Some(ip),
        _ => None,
    }
}

/// Asks every address of the local /24 network whether it runs LocalSend.
async fn scan_subnet(found: tokio::sync::mpsc::UnboundedSender<Device>) {
    let Some(own) = local_ipv4() else {
        return;
    };
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_millis(1500))
        .build()
        .expect("TLS backend available");
    let limit = Arc::new(tokio::sync::Semaphore::new(48));
    let [a, b, c, _] = own.octets();
    let mut tasks = Vec::new();
    for host in 1..=254u8 {
        let ip = Ipv4Addr::new(a, b, c, host);
        if ip == own {
            continue;
        }
        let (client, found, limit) = (client.clone(), found.clone(), limit.clone());
        tasks.push(tokio::spawn(async move {
            let _permit = limit.acquire().await;
            for https in [true, false] {
                let device = Device {
                    info: own_info(None),
                    ip: IpAddr::V4(ip),
                    port: PORT,
                    https,
                };
                let Ok(response) = client.get(device.url("/info")).send().await else {
                    continue;
                };
                if let Ok(info) = response.json::<DeviceInfo>().await {
                    let _ = found.send(Device { info, ..device });
                    return;
                }
            }
        }));
    }
    for task in tasks {
        let _ = task.await;
    }
}

/// Progress of sending to one device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendEvent {
    /// Waiting for the other side to accept.
    Waiting,
    Progress { done: usize, total: usize },
    Done,
    Rejected,
    Failed(String),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileMeta {
    id: String,
    file_name: String,
    size: u64,
    file_type: String,
}

#[derive(Serialize)]
struct PrepareRequest {
    info: DeviceInfo,
    files: std::collections::HashMap<String, FileMeta>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrepareResponse {
    session_id: String,
    files: std::collections::HashMap<String, String>,
}

/// Files to send, with the names they get on the other side; folders are
/// sent with their contents under the folder's name.
pub fn collect_files(paths: &[PathBuf]) -> Vec<(PathBuf, String)> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<(PathBuf, String)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let name = format!("{prefix}/{}", entry.file_name().to_string_lossy());
            let path = entry.path();
            if path.is_dir() {
                walk(&path, &name, out);
            } else if path.is_file() {
                out.push((path, name));
            }
        }
    }
    let mut out = Vec::new();
    for path in paths {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if path.is_dir() {
            walk(path, &name, &mut out);
        } else if path.is_file() {
            out.push((path.clone(), name));
        }
    }
    out
}

/// Sends `paths` to `device`, reporting how it goes.
pub fn send(device: Device, paths: Vec<PathBuf>) -> impl Stream<Item = SendEvent> {
    cosmic::iced::stream::channel(8, move |mut output: mpsc::Sender<SendEvent>| async move {
        let result = send_files(&device, &paths, &mut output).await;
        let last = match result {
            Ok(event) => event,
            Err(err) => SendEvent::Failed(err),
        };
        let _ = output.send(last).await;
    })
}

async fn send_files(
    device: &Device,
    paths: &[PathBuf],
    output: &mut mpsc::Sender<SendEvent>,
) -> Result<SendEvent, String> {
    let files = tokio::task::spawn_blocking({
        let paths = paths.to_vec();
        move || collect_files(&paths)
    })
    .await
    .map_err(|e| e.to_string())?;
    if files.is_empty() {
        return Ok(SendEvent::Done);
    }

    let mut metas = std::collections::HashMap::new();
    for (index, (path, name)) in files.iter().enumerate() {
        let size = tokio::fs::metadata(path).await.map(|m| m.len()).unwrap_or(0);
        let id = index.to_string();
        metas.insert(
            id.clone(),
            FileMeta {
                id,
                file_name: name.clone(),
                size,
                file_type: mime_guess::from_path(path)
                    .first_or_octet_stream()
                    .essence_str()
                    .to_owned(),
            },
        );
    }

    let _ = output.send(SendEvent::Waiting).await;
    let client = client();
    let response = client
        .post(device.url("/prepare-upload"))
        .json(&PrepareRequest {
            info: own_info(None),
            files: metas,
        })
        // The other side has this long to accept.
        .timeout(Duration::from_secs(300))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    match response.status().as_u16() {
        200 => {}
        204 => return Ok(SendEvent::Done),
        403 => return Ok(SendEvent::Rejected),
        status => return Err(format!("HTTP {status}")),
    }
    let session: PrepareResponse = response.json().await.map_err(|e| e.to_string())?;

    let total = session.files.len();
    let mut done = 0;
    let _ = output.send(SendEvent::Progress { done, total }).await;
    for (index, (path, _)) in files.iter().enumerate() {
        let id = index.to_string();
        let Some(token) = session.files.get(&id) else {
            // The receiver did not want this one.
            continue;
        };
        let file = tokio::fs::File::open(path).await.map_err(|e| e.to_string())?;
        let size = file.metadata().await.map(|m| m.len()).unwrap_or(0);
        let body = reqwest::Body::wrap_stream(tokio_util::io::ReaderStream::new(file));
        let response = client
            .post(device.url("/upload"))
            .query(&[
                ("sessionId", session.session_id.as_str()),
                ("fileId", id.as_str()),
                ("token", token.as_str()),
            ])
            .header(reqwest::header::CONTENT_LENGTH, size)
            .body(body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            let _ = client
                .post(device.url("/cancel"))
                .query(&[("sessionId", session.session_id.as_str())])
                .send()
                .await;
            return Err(format!("HTTP {}", response.status().as_u16()));
        }
        done += 1;
        let _ = output.send(SendEvent::Progress { done, total }).await;
    }
    Ok(SendEvent::Done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_announcements() {
        let json = r#"{"alias":"Pixel","version":"2.1","deviceModel":"Pixel 8","deviceType":"mobile",
            "fingerprint":"abc","port":53317,"protocol":"https","download":false,"announce":true}"#;
        let info: DeviceInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.alias, "Pixel");
        assert_eq!(info.device_type.as_deref(), Some("mobile"));
        assert_eq!(info.announce, Some(true));
    }

    #[test]
    fn parses_minimal_info() {
        // /info answers carry no port or protocol.
        let info: DeviceInfo =
            serde_json::from_str(r#"{"alias":"Mac","version":"2.0","fingerprint":"f"}"#).unwrap();
        assert_eq!(info.port, None);
    }

    #[test]
    fn own_info_is_stable_and_camel_case() {
        let a = own_info(Some(1234));
        assert_eq!(a.fingerprint, own_info(None).fingerprint);
        let json = serde_json::to_string(&a).unwrap();
        assert!(json.contains("\"deviceType\":\"desktop\""));
        assert!(json.contains("\"protocol\":\"http\""));
    }

    #[test]
    fn folders_are_sent_with_their_contents() {
        let dir = std::env::temp_dir().join(format!("island-ls-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Fotky/léto")).unwrap();
        std::fs::write(dir.join("Fotky/a.jpg"), b"a").unwrap();
        std::fs::write(dir.join("Fotky/léto/b.jpg"), b"b").unwrap();
        std::fs::write(dir.join("c.txt"), b"c").unwrap();
        let files = collect_files(&[dir.join("Fotky"), dir.join("c.txt")]);
        let names: Vec<_> = files.iter().map(|(_, n)| n.as_str()).collect();
        assert_eq!(names, ["Fotky/a.jpg", "Fotky/léto/b.jpg", "c.txt"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn urls_follow_the_protocol() {
        let device = Device {
            info: own_info(None),
            ip: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 5)),
            port: 53317,
            https: true,
        };
        assert_eq!(
            device.url("/prepare-upload"),
            "https://192.168.1.5:53317/api/localsend/v2/prepare-upload"
        );
    }

    /// Lists LocalSend devices on the network for a few seconds.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn discovers_devices() {
        use cosmic::iced::futures::StreamExt;
        let devices: Vec<Device> = discover()
            .take_until(tokio::time::sleep(Duration::from_secs(6)))
            .collect()
            .await;
        for d in &devices {
            println!("found {} ({:?}) at {}:{} https={}", d.info.alias, d.info.device_model, d.ip, d.port, d.https);
        }
    }

    /// Finds the mock receiver (`mock_localsend.py`) and sends it a folder and a file.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn sends_to_mock_receiver() {
        use cosmic::iced::futures::StreamExt;
        let device = Box::pin(
            discover().filter(|d| std::future::ready(d.info.fingerprint == "mock-fingerprint")),
        )
        .next()
        .await
        .unwrap();
        let dir = std::env::temp_dir().join(format!("island-send-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Složka")).unwrap();
        std::fs::write(dir.join("Složka/a.txt"), b"ahoj").unwrap();
        std::fs::write(dir.join("b.bin"), vec![7u8; 100_000]).unwrap();
        let events: Vec<SendEvent> =
            Box::pin(send(device, vec![dir.join("Složka"), dir.join("b.bin")]))
                .collect()
                .await;
        std::fs::remove_dir_all(dir).unwrap();
        assert_eq!(events.last(), Some(&SendEvent::Done), "{events:?}");
        assert!(events.contains(&SendEvent::Progress { done: 2, total: 2 }));
    }
}
