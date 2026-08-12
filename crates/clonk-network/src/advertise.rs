//! Host-side startup discovery and HTTP reference service matching
//! `C4Network2IODiscover` and `C4Network2RefServer`.

use std::fmt::Write as _;
use std::io;
use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6};
use std::sync::{mpsc, Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use clonk_engine::LegacyCString;
use socket2::{Protocol, Type};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};

use crate::host_game_reference::{quote_legacy, serialize_reference_parameters};
use crate::search::{join_discovery_multicast, send_discovery_datagram, DISCOVERY_MULTICAST};
use crate::{
    HostGameReference, HostGameReferenceError, NetworkAddress, NetworkGameReference,
    NetworkProtocol,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkGameAdvertiserConfig {
    pub discovery_port: u16,
    /// `None` mirrors a disabled `Config.Network.PortRefServer`. `Some(0)`
    /// remains useful for tests and embedders that need an ephemeral listener.
    pub reference_port: Option<u16>,
    /// `Config.General.LanguageCharset`, before C++ code-page canonicalization.
    pub language_charset: String,
}

fn canonical_legacy_charset_name(configured: &str) -> &'static str {
    match configured.to_ascii_uppercase().as_str() {
        "SHIFTJIS" => "CP932",
        "HANGUL" => "CP949",
        "JOHAB" => "CP1361",
        "CHINESEBIG5" => "CP950",
        "GREEK" => "CP1253",
        "TURKISH" => "CP1254",
        "VIETNAMESE" => "CP1258",
        "HEBREW" => "CP1255",
        "ARABIC" => "CP1256",
        "BALTIC" => "CP1257",
        "RUSSIAN" => "CP1251",
        "THAI" => "CP874",
        "EASTEUROPE" => "CP1250",
        "UTF-8" => "UTF-8",
        _ => "CP1252",
    }
}

/// How often a host repeats the announce it sends as it opens.
const HOST_ANNOUNCE_REPEAT_INTERVAL: Duration = Duration::from_secs(2);
/// How long a host keeps repeating that announce before falling silent.
const HOST_ANNOUNCE_REPEAT_WINDOW: Duration = Duration::from_secs(10);

/// The bounded repeat that follows a host's one oracle-exact opening announce.
///
/// It exists so a browser already on screen sees a game the moment it opens
/// rather than on its next probe, and so a single lost multicast datagram does
/// not hide the game entirely. It stops: the probe reply path answers forever,
/// exactly as C++ does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AnnounceSchedule {
    next_announce_at: Instant,
    quiet_after: Instant,
}

impl AnnounceSchedule {
    /// `started_at` is when the advertiser sent its single opening announce.
    fn started_at(started_at: Instant) -> Self {
        Self {
            next_announce_at: started_at + HOST_ANNOUNCE_REPEAT_INTERVAL,
            quiet_after: started_at + HOST_ANNOUNCE_REPEAT_WINDOW,
        }
    }

    fn take_due_announce_at(&mut self, now: Instant) -> bool {
        if now >= self.quiet_after || now < self.next_announce_at {
            return false;
        }
        self.next_announce_at = now + HOST_ANNOUNCE_REPEAT_INTERVAL;
        true
    }
}

pub fn discovery_reply_for_packet(payload: &[u8], reference_port: u16) -> Option<[u8; 4]> {
    if payload != [0x03] {
        return None;
    }
    let port = reference_port.to_ne_bytes();
    Some([0x04, 0x00, port[0], port[1]])
}

pub fn encode_reference_response(reference: &NetworkGameReference) -> Vec<u8> {
    encode_reference_response_with_charset(reference, "CP1252")
        .expect("the canonical CP1252 reference charset must be supported")
}

fn encode_reference_response_with_charset(
    reference: &NetworkGameReference,
    charset: &'static str,
) -> Result<Vec<u8>, iconv_native::ConvertLossyError> {
    let mut output = String::new();
    let _ = write!(
        output,
        "[Reference]\r\n\
Icon={}\r\n\
State={}\r\n\
CtrlMode={}\r\n\
Time={}\r\n\
StartTime={}\r\n\
Comment={}\r\n\
JoinAllowed={}\r\n\
PasswordNeeded={}\r\n",
        reference.icon,
        reference.state,
        reference.control_mode,
        reference.time,
        reference.start_time,
        quote_reference_text(&reference.comment, charset)?,
        reference.join_allowed,
        reference.password_needed,
    );
    output.push_str("Address=");
    if reference.addresses.is_empty() {
        for (index, address) in reference.tcp_addresses.iter().enumerate() {
            if index != 0 {
                output.push(',');
            }
            let _ = write!(output, "TCP:\"{address}\"");
        }
    } else {
        for (index, address) in reference.addresses.iter().enumerate() {
            if index != 0 {
                output.push(',');
            }
            match address.protocol {
                NetworkProtocol::Udp => output.push_str("UDP"),
                NetworkProtocol::Tcp => output.push_str("TCP"),
                NetworkProtocol::Unknown(protocol) => {
                    let _ = write!(output, "{protocol}");
                }
            }
            let _ = write!(output, ":\"{}\"", reference_endpoint(address));
        }
    }
    let _ = write!(
        output,
        "\r\nGame={}\r\n\
Version={},{},{},{}\r\n\
Build={}\r\n\
OfficialServer={}\r\n\
MaxPlayers={}\r\n\
UseFairCrew={}\r\n\
Goals={}\r\n\
League={}\r\n\
LeagueAddress={}\r\n\
IsNetworkGame=true\r\n\
Title={}\r\n",
        quote_reference_text(&reference.game, charset)?,
        reference.version[0],
        reference.version[1],
        reference.version[2],
        reference.version[3],
        reference.build,
        reference.official_server,
        reference.max_players,
        reference.use_fair_crew,
        quote_reference_text(
            &reference
                .goals
                .iter()
                .map(|goal| format!("{goal}=1"))
                .collect::<Vec<_>>()
                .join(";"),
            charset,
        )?,
        quote_reference_text(&reference.league, charset)?,
        quote_reference_text(&reference.league_address, charset)?,
        quote_reference_text(&reference.title, charset)?,
    );
    if !reference.player_names.is_empty() {
        output.push_str("\r\n  [PlayerInfos]\r\n");
        let _ = write!(
            output,
            "  LastPlayerID={}\r\n",
            reference.player_names.len()
        );
        output.push_str("\r\n    [Client]\r\n    ID=0\r\n    Flags=Initial\r\n");
        for (index, name) in reference.player_names.iter().enumerate() {
            output.push_str("\r\n      [Player]\r\n");
            let _ = write!(
                output,
                "      Name={}\r\n      Flags=Joined\r\n      ID={}\r\n",
                quote_reference_text(name, charset)?,
                index + 1
            );
        }
    }
    output.push_str("\r\n  [Client]\r\n  ID=0\r\n  Activated=true\r\n");
    let _ = write!(
        output,
        "  Name={}\r\n  Nick={}\r\n",
        quote_reference_text(&reference.host_name, charset)?,
        quote_reference_text(&reference.host_nick, charset)?,
    );
    if reference.netpuncher_ipv4 != 0 || reference.netpuncher_ipv6 != 0 {
        output.push_str("\r\n  [NetpuncherID]\r\n");
        if reference.netpuncher_ipv4 != 0 {
            let _ = write!(output, "  IPv4={}\r\n", reference.netpuncher_ipv4);
        }
        if reference.netpuncher_ipv6 != 0 {
            let _ = write!(output, "  IPv6={}\r\n", reference.netpuncher_ipv6);
        }
    }
    if !reference.netpuncher_address.is_empty() {
        let _ = write!(
            output,
            "NetpuncherAddr={}\r\n",
            quote_reference_text(&reference.netpuncher_address, charset)?
        );
    }

    Ok(output
        .chars()
        .map(|character| u8::try_from(u32::from(character)).unwrap_or(b'?'))
        .collect())
}

/// Serializes the host-only exact reference path. Unlike the legacy summary
/// encoder above, this requires the complete `C4GameParameters` snapshot and
/// follows `C4Network2Reference::CompileFunc` default elision and field order.
pub fn encode_host_game_reference_response(
    reference: &HostGameReference,
) -> Result<Vec<u8>, HostGameReferenceError> {
    reference.validate()?;
    let summary = reference.summary();
    let metadata = reference.metadata();
    let mut output = String::from("[Reference]\r\n");
    if metadata.icon != 0 {
        let _ = write!(output, "Icon={}\r\n", metadata.icon);
    }
    if summary.state != "None" {
        let _ = write!(output, "State={}\r\n", summary.state);
    }
    if summary.control_mode != -1 {
        let _ = write!(output, "CtrlMode={}\r\n", summary.control_mode);
    }
    if metadata.time != 0 {
        let _ = write!(output, "Time={}\r\n", metadata.time);
    }
    if metadata.frame != 0 {
        let _ = write!(output, "Frame={}\r\n", metadata.frame);
    }
    if summary.start_time != 0 {
        let _ = write!(output, "StartTime={}\r\n", summary.start_time);
    }
    if metadata.league_performance != 0 {
        let _ = write!(
            output,
            "LeaguePerformance={}\r\n",
            metadata.league_performance
        );
    }
    if !metadata.comment.is_empty() {
        let _ = write!(output, "Comment={}\r\n", quote_legacy(&metadata.comment));
    }
    if !summary.join_allowed {
        output.push_str("JoinAllowed=false\r\n");
    }
    if summary.password_needed {
        output.push_str("PasswordNeeded=true\r\n");
    }
    if !metadata.addresses.is_empty() {
        output.push_str("Address=");
        for (index, address) in metadata.addresses.iter().enumerate() {
            if index != 0 {
                output.push(',');
            }
            let protocol = match address.protocol {
                NetworkProtocol::Udp => "UDP",
                NetworkProtocol::Tcp => "TCP",
                NetworkProtocol::Unknown(_) => {
                    unreachable!("reference metadata validates address protocols")
                }
            };
            let _ = write!(output, "{protocol}:\"{}\"", reference_endpoint(address));
        }
        output.push_str("\r\n");
    }
    if summary.game != "None" {
        let _ = write!(output, "Game={}\r\n", quote_ini(&summary.game));
    }
    if let Some(last) = summary.version.iter().rposition(|value| *value != 0) {
        output.push_str("Version=");
        for (index, value) in summary.version[..=last].iter().enumerate() {
            if index != 0 {
                output.push(',');
            }
            let _ = write!(output, "{value}");
        }
        output.push_str("\r\n");
    }
    if summary.build != -1 {
        let _ = write!(output, "Build={}\r\n", summary.build);
    }
    if summary.official_server {
        output.push_str("OfficialServer=true\r\n");
    }
    serialize_reference_parameters(&mut output, reference.parameters())?;
    if metadata.netpuncher_ipv4 != 0 || metadata.netpuncher_ipv6 != 0 {
        output.push_str("\r\n  [NetpuncherID]\r\n");
        if metadata.netpuncher_ipv4 != 0 {
            let _ = write!(output, "  IPv4={}\r\n", metadata.netpuncher_ipv4);
        }
        if metadata.netpuncher_ipv6 != 0 {
            let _ = write!(output, "  IPv6={}\r\n", metadata.netpuncher_ipv6);
        }
    }
    if !metadata.netpuncher_address.is_empty() {
        let _ = write!(
            output,
            "NetpuncherAddr={}\r\n",
            quote_legacy(&metadata.netpuncher_address)
        );
    }

    Ok(output
        .chars()
        .map(|character| u8::try_from(u32::from(character)).unwrap_or(b'?'))
        .collect())
}

fn reference_endpoint(address: &NetworkAddress) -> String {
    match NetworkAddress::new(address.protocol, address.endpoint).endpoint {
        SocketAddr::V4(address) => address.to_string(),
        SocketAddr::V6(address) => format!("[{}]:{}", address.ip(), address.port()),
    }
}

fn quote_ini(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        match character {
            '\\' | '"' => {
                quoted.push('\\');
                quoted.push(character);
            }
            '\r' | '\n' => quoted.push('|'),
            _ => quoted.push(character),
        }
    }
    quoted.push('"');
    quoted
}

fn quote_reference_text(
    value: &str,
    charset: &'static str,
) -> Result<String, iconv_native::ConvertLossyError> {
    let mut encoded = iconv_native::convert_lossy(value.as_bytes(), "UTF-8", charset)?;
    if let Some(nul) = encoded.iter().position(|byte| *byte == 0) {
        encoded.truncate(nul);
    }
    let value = LegacyCString::from_bytes(encoded)
        .expect("truncating at the first NUL produces a valid legacy string");
    Ok(quote_legacy(&value))
}

pub struct NetworkGameAdvertiser {
    reference: Arc<RwLock<Vec<u8>>>,
    charset: &'static str,
    stop: mpsc::Sender<()>,
    worker: Option<thread::JoinHandle<()>>,
    reference_addr: SocketAddr,
}

impl NetworkGameAdvertiser {
    /// Legacy summary-only startup retained for incomplete callers. New host
    /// lifecycle code must use `start_exact` so game parameters are present.
    pub fn start(
        config: NetworkGameAdvertiserConfig,
        reference: NetworkGameReference,
    ) -> io::Result<Self> {
        let charset = canonical_legacy_charset_name(&config.language_charset);
        let encoded =
            encode_reference_response_with_charset(&reference, charset).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::Unsupported,
                    format!("configured reference charset {charset} is unavailable: {error}"),
                )
            })?;
        Self::start_encoded(config, charset, encoded)
    }

    pub fn start_exact(
        config: NetworkGameAdvertiserConfig,
        reference: HostGameReference,
    ) -> Result<Self, HostGameAdvertiserError> {
        let charset = canonical_legacy_charset_name(&config.language_charset);
        let encoded = encode_host_game_reference_response(&reference)?;
        Self::start_encoded(config, charset, encoded).map_err(HostGameAdvertiserError::Io)
    }

    fn start_encoded(
        config: NetworkGameAdvertiserConfig,
        charset: &'static str,
        reference: Vec<u8>,
    ) -> io::Result<Self> {
        let reference_listener = config
            .reference_port
            .map(create_reference_listener)
            .transpose()?;
        let reference_addr = reference_listener
            .as_ref()
            .map(std::net::TcpListener::local_addr)
            .transpose()?
            .unwrap_or_else(|| SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0)));
        let actual_reference_port = reference_addr.port();
        let discovery = if config.discovery_port == 0 {
            None
        } else {
            Some(create_discovery_socket(config.discovery_port)?)
        };
        let reference = Arc::new(RwLock::new(reference));
        let worker_reference = Arc::clone(&reference);
        let (stop_tx, stop_rx) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("clonk-game-advertiser".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(_) => return,
                };
                runtime.block_on(async move {
                    let listener = match reference_listener {
                        Some(listener) => match TcpListener::from_std(listener) {
                            Ok(listener) => Some(listener),
                            Err(_) => return,
                        },
                        None => None,
                    };
                    let discovery = discovery.and_then(|(socket, interfaces)| {
                        UdpSocket::from_std(socket)
                            .ok()
                            .map(|socket| (socket, interfaces))
                    });
                    run_advertiser(
                        listener,
                        discovery,
                        config.discovery_port,
                        actual_reference_port,
                        worker_reference,
                        charset,
                        stop_rx,
                    )
                    .await;
                });
            })?;
        Ok(Self {
            reference,
            charset,
            stop: stop_tx,
            worker: Some(worker),
            reference_addr,
        })
    }

    pub fn reference_addr(&self) -> SocketAddr {
        self.reference_addr
    }

    pub fn update(&self, reference: &NetworkGameReference) {
        let Ok(encoded) = encode_reference_response_with_charset(reference, self.charset) else {
            return;
        };
        if let Ok(mut current) = self.reference.write() {
            *current = encoded;
        }
    }

    pub fn update_exact(
        &self,
        reference: &HostGameReference,
    ) -> Result<(), HostGameReferenceError> {
        let encoded = encode_host_game_reference_response(reference)?;
        if let Ok(mut current) = self.reference.write() {
            *current = encoded;
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum HostGameAdvertiserError {
    #[error(transparent)]
    Reference(#[from] HostGameReferenceError),
    #[error("reference server startup failed: {0}")]
    Io(#[source] io::Error),
}

impl Drop for NetworkGameAdvertiser {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

async fn run_advertiser(
    listener: Option<TcpListener>,
    discovery: Option<(UdpSocket, Vec<u32>)>,
    discovery_port: u16,
    reference_port: u16,
    reference: Arc<RwLock<Vec<u8>>>,
    charset: &'static str,
    stop: mpsc::Receiver<()>,
) {
    if let Some(discovery) = discovery.as_ref() {
        announce(&discovery.0, &discovery.1, discovery_port, reference_port).await;
    }
    let mut announces = AnnounceSchedule::started_at(Instant::now());
    let mut datagram = [0_u8; 64];
    loop {
        match stop.try_recv() {
            Ok(()) | Err(mpsc::TryRecvError::Disconnected) => break,
            Err(mpsc::TryRecvError::Empty) => {}
        }
        if let Some(listener) = listener.as_ref() {
            if let Ok(Ok((stream, _))) =
                tokio::time::timeout(Duration::from_millis(20), listener.accept()).await
            {
                let reference = Arc::clone(&reference);
                tokio::spawn(async move {
                    serve_reference(stream, reference, charset).await;
                });
            }
        } else {
            // The TCP accept timeout normally paces discovery polling. Retain
            // that cadence when the reference server is configured off.
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        if let Some(discovery) = discovery.as_ref() {
            if announces.take_due_announce_at(Instant::now()) {
                announce(&discovery.0, &discovery.1, discovery_port, reference_port).await;
            }
            loop {
                match discovery.0.try_recv_from(&mut datagram) {
                    Ok((size, _)) => {
                        if discovery_reply_for_packet(&datagram[..size], reference_port).is_some() {
                            announce(&discovery.0, &discovery.1, discovery_port, reference_port)
                                .await;
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(_) => break,
                }
            }
        }
    }
}

async fn announce(
    discovery: &UdpSocket,
    interfaces: &[u32],
    discovery_port: u16,
    reference_port: u16,
) {
    let Some(reply) = discovery_reply_for_packet(&[0x03], reference_port) else {
        return;
    };
    let target = SocketAddrV6::new(DISCOVERY_MULTICAST, discovery_port, 0, 0);
    let _ = send_discovery_datagram(discovery, &reply, target, interfaces).await;
}

async fn serve_reference(
    mut stream: TcpStream,
    reference: Arc<RwLock<Vec<u8>>>,
    charset: &'static str,
) {
    let mut request = Vec::with_capacity(1024);
    let mut buffer = [0_u8; 1024];
    let read_request = async {
        while request.len() < 16 * 1024 {
            let size = stream.read(&mut buffer).await?;
            if size == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..size]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        io::Result::Ok(())
    };
    match tokio::time::timeout(Duration::from_secs(5), read_request).await {
        Ok(Ok(())) => {}
        Ok(Err(_)) | Err(_) => return,
    }
    if !request.windows(4).any(|window| window == b"\r\n\r\n") {
        return;
    }
    if !request.starts_with(b"GET ") {
        let _ = stream
            .write_all(b"HTTP/1.0 405 Method Not Allowed\r\n\r\n")
            .await;
        return;
    }
    let body = reference
        .read()
        .map(|reference| reference.clone())
        .unwrap_or_default();
    let header = format!(
        "HTTP/1.0 200 OK\r\n\
Content-Length: {}\r\n\
Content-Type: text/plain; charset={charset}\r\n\
Server: ClonkRust/{engine}\r\n\r\n",
        body.len(),
        engine = clonk_core::version::ENGINE_VERSION_COMPACT
    );
    let _ = stream.write_all(header.as_bytes()).await;
    let _ = stream.write_all(&body).await;
    let _ = stream.shutdown().await;
}

fn create_reference_listener(port: u16) -> io::Result<std::net::TcpListener> {
    let requested = SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, port, 0, 0));
    let (socket, address) =
        crate::dual_stack::create_bound_socket(requested, Type::STREAM, Some(Protocol::TCP))?;
    socket.set_reuse_address(true)?;
    socket.bind(&address.into())?;
    socket.listen(128)?;
    socket.set_nonblocking(true)?;
    Ok(socket.into())
}

fn create_discovery_socket(port: u16) -> io::Result<(std::net::UdpSocket, Vec<u32>)> {
    let requested = SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, port, 0, 0));
    let (socket, address) =
        crate::dual_stack::create_bound_socket(requested, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    socket.set_reuse_port(true)?;
    // C++ discovery is IPv6 multicast only (C4NetIO.cpp:1617-1633), so a host
    // without an IPv6 stack simply cannot be found on the LAN. It still has to
    // be able to host: keep the degraded socket and leave its group list empty
    // instead of failing the advertiser outright.
    let dual_stack = crate::dual_stack::bound_socket_family(address)
        == crate::dual_stack::SocketFamily::DualStack;
    if dual_stack {
        socket.set_multicast_hops_v6(16)?;
        socket.set_multicast_loop_v6(true)?;
    }
    socket.bind(&address.into())?;
    let multicast_interfaces = if dual_stack {
        join_discovery_multicast(&socket)
    } else {
        Vec::new()
    };
    socket.set_nonblocking(true)?;
    Ok((socket.into(), multicast_interfaces))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_started_host_repeats_its_discovery_announce_on_the_burst_interval() {
        // Deliberate divergence. C4Network2IO::SetAcceptMode announces exactly
        // once as the host opens (pinned oracle src/C4Network2IO.cpp:268-278;
        // src/C4Network2Discover.cpp:67-72) and the host is silent afterwards
        // until something probes it, so a single lost multicast datagram - UDP
        // does not retransmit, and Wi-Fi carries multicast unacknowledged -
        // hides the new game until the browser's own next probe. The repeat
        // gives that announce more than one chance to land.
        let started_at = Instant::now();
        let mut schedule = AnnounceSchedule::started_at(started_at);

        assert!(
            !schedule.take_due_announce_at(started_at),
            "the announce the advertiser already sent is not owed again"
        );
        assert!(!schedule.take_due_announce_at(
            started_at + HOST_ANNOUNCE_REPEAT_INTERVAL - Duration::from_millis(1)
        ));
        assert!(schedule.take_due_announce_at(started_at + HOST_ANNOUNCE_REPEAT_INTERVAL));
        assert!(schedule.take_due_announce_at(started_at + HOST_ANNOUNCE_REPEAT_INTERVAL * 2));
    }

    #[test]
    fn a_host_falls_silent_after_its_announce_burst_window() {
        // The burst covers the moment a game opens while browsers are already
        // watching; a browser opened later is covered by its own first probe.
        // It must not become a heartbeat: a C++ client opens a fresh query row
        // and re-fetches the reference for every announce it sees, because
        // AddReferenceQuery dedupes unretrieved rows only (pinned oracle
        // src/C4StartupNetDlg.cpp:1133-1154,590-600), so a host that never
        // stopped announcing would blink a query row on every C++ client on the
        // group for as long as it hosted.
        let started_at = Instant::now();
        let mut schedule = AnnounceSchedule::started_at(started_at);

        assert!(!schedule.take_due_announce_at(started_at + HOST_ANNOUNCE_REPEAT_WINDOW));
        assert!(!schedule.take_due_announce_at(started_at + Duration::from_secs(3600)));
    }
}
