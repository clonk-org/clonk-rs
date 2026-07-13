//! Host-side startup discovery and HTTP reference service matching
//! `C4Network2IODiscover` and `C4Network2RefServer`.

use std::fmt::Write as _;
use std::io;
use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6};
use std::sync::{mpsc, Arc, RwLock};
use std::thread;
use std::time::Duration;

use socket2::{Domain, Protocol, Socket, Type};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};

use crate::NetworkGameReference;

const DISCOVERY_MULTICAST: Ipv6Addr = Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetworkGameAdvertiserConfig {
    pub discovery_port: u16,
    pub reference_port: u16,
}

pub fn discovery_reply_for_packet(payload: &[u8], reference_port: u16) -> Option<[u8; 4]> {
    if payload != [0x03] {
        return None;
    }
    let port = reference_port.to_ne_bytes();
    Some([0x04, 0x00, port[0], port[1]])
}

pub fn encode_reference_response(reference: &NetworkGameReference) -> Vec<u8> {
    let mut output = String::new();
    let _ = write!(
        output,
        "[Reference]\r\n\
Icon=0\r\n\
State={}\r\n\
CtrlMode=0\r\n\
StartTime={}\r\n\
JoinAllowed={}\r\n\
PasswordNeeded={}\r\n",
        reference.state, reference.start_time, reference.join_allowed, reference.password_needed,
    );
    output.push_str("Address=");
    for (index, address) in reference.tcp_addresses.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        let _ = write!(output, "TCP:\"{address}\"");
    }
    let _ = write!(
        output,
        "\r\nGame={}\r\n\
Version={},{},{},{}\r\n\
Build={}\r\n\
OfficialServer={}\r\n\
MaxPlayers=8\r\n\
IsNetworkGame=true\r\n\
Title={}\r\n",
        quote_ini(&reference.game),
        reference.version[0],
        reference.version[1],
        reference.version[2],
        reference.version[3],
        reference.build,
        reference.official_server,
        quote_ini(&reference.title),
    );
    output.push_str("\r\n  [Client]\r\n  ID=0\r\n  Activated=true\r\n");
    let _ = write!(
        output,
        "  Name={}\r\n  Nick={}\r\n",
        quote_ini(&reference.host_name),
        quote_ini(&reference.host_name),
    );

    // The C++ reference server declares the configured legacy charset. The
    // parity server uses ISO-8859-1 and substitutes characters it cannot
    // represent, matching a lossy legacy-codepage conversion.
    output
        .chars()
        .map(|character| u8::try_from(u32::from(character)).unwrap_or(b'?'))
        .collect()
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

pub struct NetworkGameAdvertiser {
    reference: Arc<RwLock<Vec<u8>>>,
    stop: mpsc::Sender<()>,
    worker: Option<thread::JoinHandle<()>>,
    reference_addr: SocketAddr,
}

impl NetworkGameAdvertiser {
    pub fn start(
        config: NetworkGameAdvertiserConfig,
        reference: NetworkGameReference,
    ) -> io::Result<Self> {
        let reference_listener = create_reference_listener(config.reference_port)?;
        let reference_addr = reference_listener.local_addr()?;
        let actual_reference_port = reference_addr.port();
        let discovery = if config.discovery_port == 0 {
            None
        } else {
            Some(create_discovery_socket(config.discovery_port)?)
        };
        let reference = Arc::new(RwLock::new(encode_reference_response(&reference)));
        let worker_reference = Arc::clone(&reference);
        let (stop_tx, stop_rx) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("lc-game-advertiser".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(_) => return,
                };
                runtime.block_on(async move {
                    let listener = match TcpListener::from_std(reference_listener) {
                        Ok(listener) => listener,
                        Err(_) => return,
                    };
                    let discovery = discovery.and_then(|socket| UdpSocket::from_std(socket).ok());
                    run_advertiser(
                        listener,
                        discovery,
                        config.discovery_port,
                        actual_reference_port,
                        worker_reference,
                        stop_rx,
                    )
                    .await;
                });
            })?;
        Ok(Self {
            reference,
            stop: stop_tx,
            worker: Some(worker),
            reference_addr,
        })
    }

    pub fn reference_addr(&self) -> SocketAddr {
        self.reference_addr
    }

    pub fn update(&self, reference: &NetworkGameReference) {
        if let Ok(mut current) = self.reference.write() {
            *current = encode_reference_response(reference);
        }
    }
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
    listener: TcpListener,
    discovery: Option<UdpSocket>,
    discovery_port: u16,
    reference_port: u16,
    reference: Arc<RwLock<Vec<u8>>>,
    stop: mpsc::Receiver<()>,
) {
    if let Some(discovery) = discovery.as_ref() {
        announce(discovery, discovery_port, reference_port).await;
    }
    let mut datagram = [0_u8; 64];
    loop {
        match stop.try_recv() {
            Ok(()) | Err(mpsc::TryRecvError::Disconnected) => break,
            Err(mpsc::TryRecvError::Empty) => {}
        }
        if let Ok(Ok((stream, _))) =
            tokio::time::timeout(Duration::from_millis(20), listener.accept()).await
        {
            let reference = Arc::clone(&reference);
            tokio::spawn(async move {
                serve_reference(stream, reference).await;
            });
        }
        if let Some(discovery) = discovery.as_ref() {
            loop {
                match discovery.try_recv_from(&mut datagram) {
                    Ok((size, _)) => {
                        if discovery_reply_for_packet(&datagram[..size], reference_port).is_some() {
                            announce(discovery, discovery_port, reference_port).await;
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(_) => break,
                }
            }
        }
    }
}

async fn announce(discovery: &UdpSocket, discovery_port: u16, reference_port: u16) {
    let Some(reply) = discovery_reply_for_packet(&[0x03], reference_port) else {
        return;
    };
    let target = SocketAddrV6::new(DISCOVERY_MULTICAST, discovery_port, 0, 0);
    let _ = discovery.send_to(&reply, target).await;
}

async fn serve_reference(mut stream: TcpStream, reference: Arc<RwLock<Vec<u8>>>) {
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
Content-Type: text/plain; charset=ISO-8859-1\r\n\
Server: LegacyClonk/4.9.11.0 [362]\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes()).await;
    let _ = stream.write_all(&body).await;
    let _ = stream.shutdown().await;
}

fn create_reference_listener(port: u16) -> io::Result<std::net::TcpListener> {
    let socket = Socket::new(Domain::IPV6, Type::STREAM, Some(Protocol::TCP))?;
    socket.set_only_v6(false)?;
    socket.set_reuse_address(true)?;
    socket.bind(&SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, port, 0, 0).into())?;
    socket.listen(128)?;
    socket.set_nonblocking(true)?;
    Ok(socket.into())
}

fn create_discovery_socket(port: u16) -> io::Result<std::net::UdpSocket> {
    let socket = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_only_v6(false)?;
    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    socket.set_reuse_port(true)?;
    socket.set_multicast_hops_v6(16)?;
    socket.set_multicast_loop_v6(true)?;
    socket.bind(&SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, port, 0, 0).into())?;
    socket.join_multicast_v6(&DISCOVERY_MULTICAST, 0)?;
    socket.set_nonblocking(true)?;
    Ok(socket.into())
}
