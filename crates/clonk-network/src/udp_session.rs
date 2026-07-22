//! Session-facing streams over the packet-oriented reliable-UDP driver.
//!
//! `ControlTransport` speaks C4NetIOTCP's internal `0xff + native u32`
//! framing. C4NetIOUDP instead carries the complete packet body directly in
//! one reliable packet. This module owns the shared UDP socket and presents
//! each connected peer as an `AsyncRead + AsyncWrite` stream which adds that
//! framing on receive and removes it on send. The framing is therefore only
//! an in-process adapter; it is never emitted on the UDP wire.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    future::Future,
    io,
    net::SocketAddr,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    task::{Context, Poll},
};

use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use crate::{
    canonical_reliable_udp_peer_address, NetpuncherAddressFamily, NetpuncherIoEvent,
    NetpuncherPacket, NetpuncherRole, ReliableUdpDisconnectReason, ReliableUdpDriverError,
    ReliableUdpEvent, ReliableUdpSocketDriver,
};

const TCP_FRAME_PREFIX: u8 = 0xff;
const TCP_FRAME_HEADER_SIZE: usize = 5;
const HUB_COMMAND_CAPACITY: usize = 64;
const INCOMING_PEER_CAPACITY: usize = 32;
const PUNCHER_EVENT_CAPACITY: usize = 16;
const PEER_INBOUND_PACKET_CAPACITY: usize = 64;

type HubCommandPermitFuture = Pin<
    Box<
        dyn Future<Output = Result<mpsc::OwnedPermit<HubCommand>, mpsc::error::SendError<()>>>
            + Send,
    >,
>;

enum HubCommand {
    InitPuncher {
        address: SocketAddr,
        role: NetpuncherRole,
        response: oneshot::Sender<io::Result<()>>,
    },
    SendPuncherPacket {
        family: NetpuncherAddressFamily,
        packet: NetpuncherPacket,
        response: oneshot::Sender<io::Result<()>>,
    },
    ClosePuncher {
        address: SocketAddr,
        response: oneshot::Sender<io::Result<()>>,
    },
    Connect {
        peer: SocketAddr,
        response: oneshot::Sender<io::Result<ReliableUdpPeerStream>>,
    },
    BindStatistics {
        peer: SocketAddr,
        generation: u64,
        connection_id: u32,
        response: oneshot::Sender<io::Result<()>>,
    },
    Send {
        peer: SocketAddr,
        generation: u64,
        payload: Vec<u8>,
    },
    Close {
        peer: SocketAddr,
        generation: u64,
    },
    Shutdown {
        completion: Option<oneshot::Sender<()>>,
    },
}

enum PeerInbound {
    Packet(Vec<u8>),
    Disconnected(ReliableUdpDisconnectReason),
    Failed(String),
}

#[derive(Clone)]
enum PeerTerminal {
    Disconnected(ReliableUdpDisconnectReason),
    Failed(String),
    Closed,
}

struct PeerTerminalState {
    closed: AtomicBool,
    reason: Mutex<Option<PeerTerminal>>,
}

impl PeerTerminalState {
    fn open() -> Self {
        Self {
            closed: AtomicBool::new(false),
            reason: Mutex::new(None),
        }
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    fn close(&self, reason: PeerTerminal) {
        let mut retained = self.reason.lock().expect("peer terminal state poisoned");
        if retained.is_none() {
            *retained = Some(reason);
        }
        self.closed.store(true, Ordering::Release);
    }

    fn reason(&self) -> Option<PeerTerminal> {
        self.reason
            .lock()
            .expect("peer terminal state poisoned")
            .clone()
    }
}

fn terminal_read_result(terminal: Option<PeerTerminal>) -> io::Result<()> {
    match terminal {
        Some(PeerTerminal::Disconnected(reason)) => match reason {
            ReliableUdpDisconnectReason::Closed | ReliableUdpDisconnectReason::ClosedByPeer => {
                Ok(())
            }
            ReliableUdpDisconnectReason::ConnectionTimeout => {
                Err(io::Error::new(io::ErrorKind::TimedOut, reason.as_str()))
            }
            ReliableUdpDisconnectReason::ConnectionReset
            | ReliableUdpDisconnectReason::Reconnect => Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                reason.as_str(),
            )),
            ReliableUdpDisconnectReason::Starvation => Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                reason.as_str(),
            )),
        },
        Some(PeerTerminal::Failed(error)) => {
            Err(io::Error::new(io::ErrorKind::ConnectionAborted, error))
        }
        Some(PeerTerminal::Closed) | None => Ok(()),
    }
}

struct ConnectedPeer {
    generation: u64,
    inbound: mpsc::Sender<PeerInbound>,
    terminal: Arc<PeerTerminalState>,
}

/// One reliable-UDP peer exposed through the stream contract expected by
/// `ControlTransport`.
pub struct ReliableUdpPeerStream {
    peer: SocketAddr,
    generation: u64,
    commands: mpsc::Sender<HubCommand>,
    inbound: mpsc::Receiver<PeerInbound>,
    terminal: Arc<PeerTerminalState>,
    read_frame: Vec<u8>,
    read_offset: usize,
    write_buffer: Vec<u8>,
    pending_send: Option<Vec<u8>>,
    send_reservation: Option<HubCommandPermitFuture>,
    read_closed: bool,
    write_closed: bool,
    close_requested: bool,
}

impl ReliableUdpPeerStream {
    fn new(
        peer: SocketAddr,
        generation: u64,
        commands: mpsc::Sender<HubCommand>,
        inbound: mpsc::Receiver<PeerInbound>,
        terminal: Arc<PeerTerminalState>,
    ) -> Self {
        Self {
            peer,
            generation,
            commands,
            inbound,
            terminal,
            read_frame: Vec::new(),
            read_offset: 0,
            write_buffer: Vec::new(),
            pending_send: None,
            send_reservation: None,
            read_closed: false,
            write_closed: false,
            close_requested: false,
        }
    }

    pub fn peer_addr(&self) -> SocketAddr {
        self.peer
    }

    /// Binds this physical UDP peer to its high-level Network2 connection ID.
    /// The hub keeps accounting below the synthetic stream framing layer.
    pub fn bind_statistics_connection(
        &self,
        connection_id: u32,
    ) -> impl Future<Output = io::Result<()>> + Send + 'static {
        let terminal_closed = self.terminal.is_closed();
        let commands = self.commands.clone();
        let peer = self.peer;
        let generation = self.generation;
        async move {
            if terminal_closed {
                return Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    "reliable-UDP peer stream is closed",
                ));
            }
            let (response, completed) = oneshot::channel();
            commands
                .send(HubCommand::BindStatistics {
                    peer,
                    generation,
                    connection_id,
                    response,
                })
                .await
                .map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "reliable-UDP session hub stopped while binding statistics",
                    )
                })?;
            completed.await.map_err(|_| {
                io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "reliable-UDP session hub stopped while binding statistics",
                )
            })?
        }
    }

    fn buffered_frame_size(&self) -> io::Result<Option<usize>> {
        if self.write_buffer.is_empty() {
            return Ok(None);
        }
        if self.write_buffer[0] != TCP_FRAME_PREFIX {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "reliable-UDP session stream received an invalid TCP frame prefix",
            ));
        }
        if self.write_buffer.len() < TCP_FRAME_HEADER_SIZE {
            return Ok(None);
        }
        let packet_size = u32::from_ne_bytes(
            self.write_buffer[1..TCP_FRAME_HEADER_SIZE]
                .try_into()
                .expect("TCP frame header length checked"),
        ) as usize;
        TCP_FRAME_HEADER_SIZE
            .checked_add(packet_size)
            .map(Some)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "TCP frame size overflow"))
    }

    fn poll_pending_send(&mut self, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.terminal.is_closed() {
            self.mark_transport_closed();
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "reliable-UDP peer stream is closed",
            )));
        }
        if self.pending_send.is_none() {
            return Poll::Ready(Ok(()));
        }
        if self.send_reservation.is_none() {
            self.send_reservation = Some(Box::pin(self.commands.clone().reserve_owned()));
        }
        let reservation = self
            .send_reservation
            .as_mut()
            .expect("pending send has a command reservation");
        match reservation.as_mut().poll(context) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(permit)) => {
                self.send_reservation = None;
                let payload = self.pending_send.take().expect("pending send payload");
                permit.send(HubCommand::Send {
                    peer: self.peer,
                    generation: self.generation,
                    payload,
                });
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Err(_)) => {
                self.send_reservation = None;
                self.pending_send = None;
                self.mark_transport_closed();
                Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "reliable-UDP session hub stopped",
                )))
            }
        }
    }

    fn mark_transport_closed(&mut self) {
        self.terminal.close(PeerTerminal::Closed);
        self.read_closed = true;
        self.write_closed = true;
        self.write_buffer.clear();
        self.pending_send = None;
        self.send_reservation = None;
        self.inbound.close();
        self.request_close();
    }

    fn request_close(&mut self) {
        if self.close_requested {
            return;
        }
        self.close_requested = true;
        let _ = self.commands.try_send(HubCommand::Close {
            peer: self.peer,
            generation: self.generation,
        });
    }

    fn install_read_frame(&mut self, payload: Vec<u8>) -> io::Result<()> {
        let packet_size = u32::try_from(payload.len()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "reliable-UDP session packet length exceeds u32",
            )
        })?;
        let frame_size = TCP_FRAME_HEADER_SIZE
            .checked_add(payload.len())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "TCP frame size overflow"))?;
        self.read_frame = Vec::with_capacity(frame_size);
        self.read_frame.push(TCP_FRAME_PREFIX);
        self.read_frame
            .extend_from_slice(&packet_size.to_ne_bytes());
        self.read_frame.extend(payload);
        self.read_offset = 0;
        Ok(())
    }
}

impl fmt::Debug for ReliableUdpPeerStream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReliableUdpPeerStream")
            .field("peer", &self.peer)
            .field("read_closed", &self.read_closed)
            .field("write_closed", &self.write_closed)
            .finish_non_exhaustive()
    }
}

impl AsyncRead for ReliableUdpPeerStream {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        output: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        loop {
            if this.read_offset < this.read_frame.len() {
                let available = &this.read_frame[this.read_offset..];
                let copy_length = available.len().min(output.remaining());
                output.put_slice(&available[..copy_length]);
                this.read_offset += copy_length;
                if this.read_offset == this.read_frame.len() {
                    this.read_frame.clear();
                    this.read_offset = 0;
                }
                return Poll::Ready(Ok(()));
            }
            if this.read_closed {
                return Poll::Ready(Ok(()));
            }
            match Pin::new(&mut this.inbound).poll_recv(context) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(PeerInbound::Packet(payload))) => {
                    if let Err(error) = this.install_read_frame(payload) {
                        this.mark_transport_closed();
                        return Poll::Ready(Err(error));
                    }
                }
                Poll::Ready(Some(PeerInbound::Disconnected(reason))) => {
                    this.mark_transport_closed();
                    return Poll::Ready(terminal_read_result(Some(PeerTerminal::Disconnected(
                        reason,
                    ))));
                }
                Poll::Ready(Some(PeerInbound::Failed(error))) => {
                    this.mark_transport_closed();
                    return Poll::Ready(terminal_read_result(Some(PeerTerminal::Failed(error))));
                }
                Poll::Ready(None) => {
                    let terminal = this.terminal.reason();
                    this.mark_transport_closed();
                    return Poll::Ready(terminal_read_result(terminal));
                }
            }
        }
    }
}

impl AsyncWrite for ReliableUdpPeerStream {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        input: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if this.write_closed || this.terminal.is_closed() {
            this.mark_transport_closed();
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "reliable-UDP peer stream is closed",
            )));
        }
        if input.is_empty() {
            return Poll::Ready(Ok(0));
        }
        match this.poll_pending_send(context) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
            Poll::Ready(Ok(())) => {}
        }

        let mut accepted = 0;
        if this.write_buffer.len() < TCP_FRAME_HEADER_SIZE {
            let header_remaining = TCP_FRAME_HEADER_SIZE - this.write_buffer.len();
            let take = header_remaining.min(input.len());
            this.write_buffer.extend_from_slice(&input[..take]);
            accepted += take;
        }
        let frame_size = match this.buffered_frame_size() {
            Ok(Some(frame_size)) => frame_size,
            Ok(None) => return Poll::Ready(Ok(accepted)),
            Err(error) => {
                this.mark_transport_closed();
                return Poll::Ready(Err(error));
            }
        };
        let frame_remaining = frame_size - this.write_buffer.len();
        let take = frame_remaining.min(input.len() - accepted);
        this.write_buffer
            .extend_from_slice(&input[accepted..accepted + take]);
        accepted += take;
        if this.write_buffer.len() == frame_size {
            let mut payload = std::mem::take(&mut this.write_buffer);
            payload.drain(..TCP_FRAME_HEADER_SIZE);
            this.pending_send = Some(payload);
            if let Poll::Ready(Err(error)) = this.poll_pending_send(context) {
                return Poll::Ready(Err(error));
            }
        }
        Poll::Ready(Ok(accepted))
    }

    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        match this.poll_pending_send(context) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
            Poll::Ready(Ok(())) => {}
        }
        if !this.write_buffer.is_empty() {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "reliable-UDP peer stream has an incomplete TCP frame",
            )));
        }
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.write_closed {
            return Poll::Ready(Ok(()));
        }
        match this.poll_pending_send(context) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
            Poll::Ready(Ok(())) => {}
        }
        if !this.write_buffer.is_empty() {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "reliable-UDP peer stream has an incomplete TCP frame",
            )));
        }
        this.read_closed = true;
        this.write_closed = true;
        this.inbound.close();
        this.request_close();
        Poll::Ready(Ok(()))
    }
}

impl Drop for ReliableUdpPeerStream {
    fn drop(&mut self) {
        self.request_close();
    }
}

/// A single peer stream which owns the socket hub that drives it.
///
/// This is the convenient client-side form: it can be moved directly into a
/// `ControlTransport` without retaining a separate hub handle. Field order is
/// intentional—the peer requests a clean Close before the hub's drop path
/// requests task shutdown.
pub struct ReliableUdpOwnedPeerStream {
    peer: ReliableUdpPeerStream,
    hub: ReliableUdpSessionHub,
}

impl ReliableUdpOwnedPeerStream {
    pub fn peer_addr(&self) -> SocketAddr {
        self.peer.peer_addr()
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.hub.local_addr()
    }
}

impl fmt::Debug for ReliableUdpOwnedPeerStream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReliableUdpOwnedPeerStream")
            .field("peer", &self.peer_addr())
            .field("local", &self.local_addr())
            .finish_non_exhaustive()
    }
}

impl AsyncRead for ReliableUdpOwnedPeerStream {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        output: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().peer).poll_read(context, output)
    }
}

impl AsyncWrite for ReliableUdpOwnedPeerStream {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        input: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().peer).poll_write(context, input)
    }

    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().peer).poll_flush(context)
    }

    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().peer).poll_shutdown(context)
    }
}

/// Cloneable command side of a shared reliable-UDP socket. Keeping this
/// separate from the hub owner lets client dial races, reconnects, and the
/// netpuncher all retain the same NAT-visible source port.
#[derive(Clone, Debug)]
pub struct ReliableUdpSessionHandle {
    commands: mpsc::Sender<HubCommand>,
}

impl ReliableUdpSessionHandle {
    pub async fn init_puncher(
        &self,
        address: SocketAddr,
        role: NetpuncherRole,
    ) -> io::Result<()> {
        let (response, completed) = oneshot::channel();
        self.commands
            .send(HubCommand::InitPuncher {
                address,
                role,
                response,
            })
            .await
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "reliable-UDP session hub stopped during puncher initialization",
                )
            })?;
        completed.await.map_err(|_| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "reliable-UDP session hub stopped during puncher initialization",
            )
        })?
    }

    pub async fn send_puncher_packet(
        &self,
        family: NetpuncherAddressFamily,
        packet: NetpuncherPacket,
    ) -> io::Result<()> {
        let (response, completed) = oneshot::channel();
        self.commands
            .send(HubCommand::SendPuncherPacket {
                family,
                packet,
                response,
            })
            .await
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "reliable-UDP session hub stopped while sending a puncher packet",
                )
            })?;
        completed.await.map_err(|_| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "reliable-UDP session hub stopped while sending a puncher packet",
            )
        })?
    }

    pub async fn close_puncher(&self, address: SocketAddr) -> io::Result<()> {
        let (response, completed) = oneshot::channel();
        self.commands
            .send(HubCommand::ClosePuncher { address, response })
            .await
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "reliable-UDP session hub stopped while closing a puncher route",
                )
            })?;
        completed.await.map_err(|_| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "reliable-UDP session hub stopped while closing a puncher route",
            )
        })?
    }

    pub async fn connect(&self, peer: SocketAddr) -> io::Result<ReliableUdpPeerStream> {
        let (response, connected) = oneshot::channel();
        self.commands
            .send(HubCommand::Connect {
                peer: canonical_reliable_udp_peer_address(peer),
                response,
            })
            .await
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "reliable-UDP session hub stopped",
                )
            })?;
        connected.await.map_err(|_| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "reliable-UDP session hub stopped during connect",
            )
        })?
    }
}

/// Shared reliable-UDP socket with stream-like outgoing and incoming peers.
pub struct ReliableUdpSessionHub {
    local_addr: SocketAddr,
    commands: mpsc::Sender<HubCommand>,
    incoming: mpsc::Receiver<io::Result<ReliableUdpPeerStream>>,
    puncher_events: Option<mpsc::Receiver<NetpuncherIoEvent>>,
    task: Option<JoinHandle<io::Result<()>>>,
    shutdown_requested: bool,
}

impl ReliableUdpSessionHub {
    pub fn bind(bind_address: SocketAddr) -> io::Result<Self> {
        Self::from_driver(ReliableUdpSocketDriver::bind(bind_address)?)
    }

    pub fn bind_with_statistics(
        bind_address: SocketAddr,
        statistics: crate::NetworkIoStatistics,
    ) -> io::Result<Self> {
        Self::from_driver(ReliableUdpSocketDriver::bind_with_statistics(
            bind_address,
            statistics,
        )?)
    }

    pub fn from_driver(driver: ReliableUdpSocketDriver) -> io::Result<Self> {
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
            io::Error::new(
                io::ErrorKind::Other,
                "reliable-UDP session hub requires an entered Tokio runtime",
            )
        })?;
        let local_addr = canonical_reliable_udp_peer_address(driver.local_addr()?);
        let (commands, command_rx) = mpsc::channel(HUB_COMMAND_CAPACITY);
        let (incoming_tx, incoming) = mpsc::channel(INCOMING_PEER_CAPACITY);
        let (puncher_event_tx, puncher_events) = mpsc::channel(PUNCHER_EVENT_CAPACITY);
        let task_commands = commands.clone();
        let task = runtime.spawn(run_hub(
            driver,
            task_commands,
            command_rx,
            incoming_tx,
            puncher_event_tx,
        ));
        Ok(Self {
            local_addr,
            commands,
            incoming,
            puncher_events: Some(puncher_events),
            task: Some(task),
            shutdown_requested: false,
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn handle(&self) -> ReliableUdpSessionHandle {
        ReliableUdpSessionHandle {
            commands: self.commands.clone(),
        }
    }

    pub async fn init_puncher(
        &self,
        address: SocketAddr,
        role: NetpuncherRole,
    ) -> io::Result<()> {
        self.handle().init_puncher(address, role).await
    }

    pub async fn send_puncher_packet(
        &self,
        family: NetpuncherAddressFamily,
        packet: NetpuncherPacket,
    ) -> io::Result<()> {
        self.handle().send_puncher_packet(family, packet).await
    }

    pub async fn close_puncher(&self, address: SocketAddr) -> io::Result<()> {
        self.handle().close_puncher(address).await
    }

    /// Detaches the low-volume callback stream so a session loop can select
    /// it independently from ordinary incoming game peers.
    pub fn take_puncher_event_receiver(
        &mut self,
    ) -> mpsc::Receiver<NetpuncherIoEvent> {
        self.puncher_events
            .take()
            .expect("puncher event receiver already taken")
    }

    pub async fn connect(&self, peer: SocketAddr) -> io::Result<ReliableUdpPeerStream> {
        self.handle().connect(peer).await
    }

    /// Connects one peer and transfers ownership of this hub into the returned
    /// stream. Dropping that stream closes the peer and shuts down the hub.
    pub async fn connect_owned(self, peer: SocketAddr) -> io::Result<ReliableUdpOwnedPeerStream> {
        let stream = self.connect(peer).await?;
        Ok(ReliableUdpOwnedPeerStream {
            peer: stream,
            hub: self,
        })
    }

    pub async fn accept(&mut self) -> io::Result<ReliableUdpPeerStream> {
        self.incoming.recv().await.unwrap_or_else(|| {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "reliable-UDP session hub stopped",
            ))
        })
    }

    pub async fn shutdown(mut self) -> io::Result<()> {
        let (completion, completed) = oneshot::channel();
        let command_delivered = self
            .commands
            .send(HubCommand::Shutdown {
                completion: Some(completion),
            })
            .await
            .is_ok();
        // If this future is cancelled while the bounded queue is full, leave
        // the flag clear so Drop can retry or abort the socket task.
        self.shutdown_requested = command_delivered;
        if command_delivered {
            let _ = completed.await;
        }
        match self.task.take() {
            Some(task) => task.await.map_err(|error| {
                io::Error::new(
                    io::ErrorKind::Other,
                    format!("reliable-UDP session task failed: {error}"),
                )
            })?,
            None => Ok(()),
        }
    }
}

impl fmt::Debug for ReliableUdpSessionHub {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReliableUdpSessionHub")
            .field("local_addr", &self.local_addr)
            .field("shutdown_requested", &self.shutdown_requested)
            .finish_non_exhaustive()
    }
}

impl Drop for ReliableUdpSessionHub {
    fn drop(&mut self) {
        if !self.shutdown_requested {
            self.shutdown_requested = true;
            if self
                .commands
                .try_send(HubCommand::Shutdown { completion: None })
                .is_err()
            {
                if let Some(task) = self.task.as_ref() {
                    task.abort();
                }
            }
        }
    }
}

async fn run_hub(
    mut driver: ReliableUdpSocketDriver,
    commands: mpsc::Sender<HubCommand>,
    mut command_rx: mpsc::Receiver<HubCommand>,
    incoming: mpsc::Sender<io::Result<ReliableUdpPeerStream>>,
    puncher_events: mpsc::Sender<NetpuncherIoEvent>,
) -> io::Result<()> {
    let mut peers = BTreeMap::<SocketAddr, ConnectedPeer>::new();
    let mut pending_connects =
        BTreeMap::<SocketAddr, oneshot::Sender<io::Result<ReliableUdpPeerStream>>>::new();
    let mut next_peer_generation = 0_u64;

    loop {
        close_abandoned_peers(&mut driver, &mut peers).await;
        tokio::select! {
            command = command_rx.recv() => {
                let Some(command) = command else {
                    close_all(&mut driver, &mut peers, &mut pending_connects).await;
                    return Ok(());
                };
                match command {
                    HubCommand::InitPuncher { address, role, response } => {
                        let result = match driver.init_puncher(address, role).await {
                            Ok(events) => {
                                dispatch_events(
                                    &mut driver,
                                    events,
                                    &commands,
                                    &incoming,
                                    &puncher_events,
                                    &mut peers,
                                    &mut pending_connects,
                                    &mut next_peer_generation,
                                )
                                .await;
                                Ok(())
                            }
                            Err(error) => Err(error),
                        };
                        let _ = response.send(result);
                    }
                    HubCommand::SendPuncherPacket { family, packet, response } => {
                        let result = match driver.send_puncher_packet(family, &packet).await {
                            Ok(events) => {
                                dispatch_events(
                                    &mut driver,
                                    events,
                                    &commands,
                                    &incoming,
                                    &puncher_events,
                                    &mut peers,
                                    &mut pending_connects,
                                    &mut next_peer_generation,
                                )
                                .await;
                                Ok(())
                            }
                            Err(error) => Err(reliable_udp_driver_io_error(error)),
                        };
                        let _ = response.send(result);
                    }
                    HubCommand::ClosePuncher { address, response } => {
                        let result = match driver.close_puncher(address).await {
                            Ok(events) => {
                                dispatch_events(
                                    &mut driver,
                                    events,
                                    &commands,
                                    &incoming,
                                    &puncher_events,
                                    &mut peers,
                                    &mut pending_connects,
                                    &mut next_peer_generation,
                                )
                                .await;
                                Ok(())
                            }
                            Err(error) => Err(error),
                        };
                        let _ = response.send(result);
                    }
                    HubCommand::Connect { peer, response } => {
                        let peer = canonical_reliable_udp_peer_address(peer);
                        if peers.contains_key(&peer)
                            || pending_connects.contains_key(&peer)
                            || driver.core().peer_status(peer).is_some()
                        {
                            let _ = response.send(Err(io::Error::new(
                                io::ErrorKind::AlreadyExists,
                                format!("reliable-UDP peer {peer} is already connected or connecting"),
                            )));
                            continue;
                        }
                        pending_connects.insert(peer, response);
                        match driver.connect(peer).await {
                            Ok(events) => {
                                dispatch_events(
                                    &mut driver,
                                    events,
                                    &commands,
                                    &incoming,
                                    &puncher_events,
                                    &mut peers,
                                    &mut pending_connects,
                                    &mut next_peer_generation,
                                )
                                .await;
                            }
                            Err(error) => {
                                fail_pending_connect(&mut pending_connects, peer, error);
                            }
                        }
                    }
                    HubCommand::BindStatistics {
                        peer,
                        generation,
                        connection_id,
                        response,
                    } => {
                        let peer = canonical_reliable_udp_peer_address(peer);
                        let result = if peer_generation_matches(&peers, peer, generation) {
                            driver.bind_peer_statistics(peer, connection_id)
                        } else {
                            Err(io::Error::new(
                                io::ErrorKind::NotConnected,
                                format!("reliable-UDP peer {peer} is no longer connected"),
                            ))
                        };
                        let _ = response.send(result);
                    }
                    HubCommand::Send {
                        peer,
                        generation,
                        payload,
                    } => {
                        let peer = canonical_reliable_udp_peer_address(peer);
                        if !peer_generation_matches(&peers, peer, generation) {
                            continue;
                        }
                        match driver.send_packet(peer, &payload).await {
                            Ok(events) => {
                                dispatch_events(
                                    &mut driver,
                                    events,
                                    &commands,
                                    &incoming,
                                    &puncher_events,
                                    &mut peers,
                                    &mut pending_connects,
                                    &mut next_peer_generation,
                                )
                                .await;
                            }
                            Err(error) => {
                                fail_peer(&mut peers, peer, generation, error.to_string());
                                let _ = driver.close_peer(peer).await;
                            }
                        }
                    }
                    HubCommand::Close { peer, generation } => {
                        let peer = canonical_reliable_udp_peer_address(peer);
                        if !peer_generation_matches(&peers, peer, generation) {
                            continue;
                        }
                        match driver.close_peer(peer).await {
                            Ok(events) => {
                                dispatch_events(
                                    &mut driver,
                                    events,
                                    &commands,
                                    &incoming,
                                    &puncher_events,
                                    &mut peers,
                                    &mut pending_connects,
                                    &mut next_peer_generation,
                                )
                                .await;
                            }
                            Err(error) => {
                                fail_peer(&mut peers, peer, generation, error.to_string())
                            }
                        }
                    }
                    HubCommand::Shutdown { completion } => {
                        close_all(&mut driver, &mut peers, &mut pending_connects).await;
                        if let Some(completion) = completion {
                            let _ = completion.send(());
                        }
                        return Ok(());
                    }
                }
            }
            ready = driver.wait_ready() => {
                // Once readiness advances the reliable-UDP core, finish its
                // ACK/event flush outside the cancellable select future.
                let result = driver.process_ready(ready).await;
                match result {
                    Ok(events) => {
                        dispatch_events(
                            &mut driver,
                            events,
                            &commands,
                            &incoming,
                            &puncher_events,
                            &mut peers,
                            &mut pending_connects,
                            &mut next_peer_generation,
                        )
                        .await;
                    }
                    Err(error) => {
                        let message = error.to_string();
                        fail_all(&mut peers, &mut pending_connects, &message);
                        let _ = incoming.try_send(Err(io::Error::new(error.kind(), message)));
                        return Err(error);
                    }
                }
            }
        }
    }
}

async fn dispatch_events(
    driver: &mut ReliableUdpSocketDriver,
    events: Vec<ReliableUdpEvent>,
    commands: &mpsc::Sender<HubCommand>,
    incoming: &mpsc::Sender<io::Result<ReliableUdpPeerStream>>,
    puncher_events: &mpsc::Sender<NetpuncherIoEvent>,
    peers: &mut BTreeMap<SocketAddr, ConnectedPeer>,
    pending_connects: &mut BTreeMap<SocketAddr, oneshot::Sender<io::Result<ReliableUdpPeerStream>>>,
    next_peer_generation: &mut u64,
) {
    for event in events {
        match event {
            ReliableUdpEvent::Connected { peer, .. } => {
                let peer = canonical_reliable_udp_peer_address(peer);
                if peers.contains_key(&peer) {
                    continue;
                }
                let generation = *next_peer_generation;
                *next_peer_generation = next_peer_generation.wrapping_add(1);
                let (inbound, inbound_rx) = mpsc::channel(PEER_INBOUND_PACKET_CAPACITY);
                let terminal = Arc::new(PeerTerminalState::open());
                let stream = ReliableUdpPeerStream::new(
                    peer,
                    generation,
                    commands.clone(),
                    inbound_rx,
                    terminal.clone(),
                );
                let delivered = if let Some(response) = pending_connects.remove(&peer) {
                    response.send(Ok(stream)).is_ok()
                } else {
                    incoming.try_send(Ok(stream)).is_ok()
                };
                if delivered {
                    peers.insert(
                        peer,
                        ConnectedPeer {
                            generation,
                            inbound,
                            terminal,
                        },
                    );
                } else {
                    let _ = driver.close_peer(peer).await;
                }
            }
            ReliableUdpEvent::Packet { peer, payload } => {
                let peer = canonical_reliable_udp_peer_address(peer);
                if let Some(connected) = peers.get(&peer) {
                    // One socket task drives every peer, so waiting here would
                    // let one stalled consumer block CHECKs and commands for
                    // all peers. The bounded queue absorbs normal bursts; a
                    // full queue deterministically fails this route instead of
                    // dropping a packet and continuing with a corrupt stream.
                    let delivery = connected.inbound.try_send(PeerInbound::Packet(payload));
                    if let Err(error) = delivery {
                        match error {
                            mpsc::error::TrySendError::Full(_) => {
                                let message = "reliable-UDP peer inbound queue is saturated";
                                connected
                                    .terminal
                                    .close(PeerTerminal::Failed(message.to_string()));
                                let _ = connected
                                    .inbound
                                    .try_send(PeerInbound::Failed(message.to_string()));
                            }
                            mpsc::error::TrySendError::Closed(_) => {
                                connected.terminal.close(PeerTerminal::Closed);
                            }
                        }
                        peers.remove(&peer);
                        let _ = driver.close_peer(peer).await;
                    }
                }
            }
            ReliableUdpEvent::Disconnected { peer, reason } => {
                let peer = canonical_reliable_udp_peer_address(peer);
                if let Some(response) = pending_connects.remove(&peer) {
                    let _ = response.send(Err(disconnect_error(peer, reason)));
                }
                if let Some(connected) = peers.remove(&peer) {
                    connected.terminal.close(PeerTerminal::Disconnected(reason));
                    let _ = connected
                        .inbound
                        .try_send(PeerInbound::Disconnected(reason));
                }
            }
            ReliableUdpEvent::Puncher(event) => {
                let puncher_address = event.puncher_address();
                if puncher_events.try_send(event).is_err() {
                    // Puncher input is network-driven. If the bounded callback
                    // queue cannot retain it, close this exact special route
                    // instead of leaking memory or misrouting later packets.
                    let _ = driver.close_puncher(puncher_address).await;
                }
            }
        }
    }
}

fn reliable_udp_driver_io_error(error: ReliableUdpDriverError) -> io::Error {
    match error {
        ReliableUdpDriverError::Io(error) => error,
        ReliableUdpDriverError::Runtime(error) => {
            io::Error::new(io::ErrorKind::InvalidData, error.to_string())
        }
    }
}

fn disconnect_error(peer: SocketAddr, reason: ReliableUdpDisconnectReason) -> io::Error {
    let kind = match reason {
        ReliableUdpDisconnectReason::ConnectionTimeout => io::ErrorKind::TimedOut,
        ReliableUdpDisconnectReason::ConnectionReset | ReliableUdpDisconnectReason::Reconnect => {
            io::ErrorKind::ConnectionReset
        }
        ReliableUdpDisconnectReason::Starvation => io::ErrorKind::ConnectionAborted,
        ReliableUdpDisconnectReason::Closed | ReliableUdpDisconnectReason::ClosedByPeer => {
            io::ErrorKind::ConnectionAborted
        }
    };
    io::Error::new(
        kind,
        format!("reliable-UDP peer {peer} disconnected: {}", reason.as_str()),
    )
}

fn fail_pending_connect(
    pending_connects: &mut BTreeMap<SocketAddr, oneshot::Sender<io::Result<ReliableUdpPeerStream>>>,
    peer: SocketAddr,
    error: io::Error,
) {
    if let Some(response) = pending_connects.remove(&peer) {
        let _ = response.send(Err(error));
    }
}

fn peer_generation_matches(
    peers: &BTreeMap<SocketAddr, ConnectedPeer>,
    peer: SocketAddr,
    generation: u64,
) -> bool {
    peers
        .get(&canonical_reliable_udp_peer_address(peer))
        .is_some_and(|connected| connected.generation == generation)
}

async fn close_abandoned_peers(
    driver: &mut ReliableUdpSocketDriver,
    peers: &mut BTreeMap<SocketAddr, ConnectedPeer>,
) {
    let abandoned = peers
        .iter()
        .filter_map(|(peer, connected)| connected.inbound.is_closed().then_some(*peer))
        .collect::<Vec<_>>();
    for peer in abandoned {
        peers.remove(&peer);
        let _ = driver.close_peer(peer).await;
    }
}

fn fail_peer(
    peers: &mut BTreeMap<SocketAddr, ConnectedPeer>,
    peer: SocketAddr,
    generation: u64,
    error: String,
) {
    let peer = canonical_reliable_udp_peer_address(peer);
    if !peer_generation_matches(peers, peer, generation) {
        return;
    }
    if let Some(connected) = peers.remove(&peer) {
        connected
            .terminal
            .close(PeerTerminal::Failed(error.clone()));
        let _ = connected.inbound.try_send(PeerInbound::Failed(error));
    }
}

fn fail_all(
    peers: &mut BTreeMap<SocketAddr, ConnectedPeer>,
    pending_connects: &mut BTreeMap<SocketAddr, oneshot::Sender<io::Result<ReliableUdpPeerStream>>>,
    error: &str,
) {
    for (_, connected) in std::mem::take(peers) {
        connected
            .terminal
            .close(PeerTerminal::Failed(error.to_string()));
        let _ = connected
            .inbound
            .try_send(PeerInbound::Failed(error.to_string()));
    }
    for (peer, response) in std::mem::take(pending_connects) {
        let _ = response.send(Err(io::Error::new(
            io::ErrorKind::ConnectionAborted,
            format!("reliable-UDP peer {peer} failed: {error}"),
        )));
    }
}

async fn close_all(
    driver: &mut ReliableUdpSocketDriver,
    peers: &mut BTreeMap<SocketAddr, ConnectedPeer>,
    pending_connects: &mut BTreeMap<SocketAddr, oneshot::Sender<io::Result<ReliableUdpPeerStream>>>,
) {
    let addresses = peers
        .keys()
        .chain(pending_connects.keys())
        .copied()
        .collect::<BTreeSet<_>>();
    for peer in addresses {
        if let Ok(events) = driver.close_peer(peer).await {
            for event in events {
                if let ReliableUdpEvent::Disconnected { peer, reason } = event {
                    let peer = canonical_reliable_udp_peer_address(peer);
                    if let Some(response) = pending_connects.remove(&peer) {
                        let _ = response.send(Err(disconnect_error(peer, reason)));
                    }
                    if let Some(connected) = peers.remove(&peer) {
                        connected.terminal.close(PeerTerminal::Disconnected(reason));
                        let _ = connected
                            .inbound
                            .try_send(PeerInbound::Disconnected(reason));
                    }
                }
            }
        }
    }
    fail_all(
        peers,
        pending_connects,
        "reliable-UDP session hub shut down",
    );
}

#[cfg(test)]
mod tests {
    use std::{net::Ipv4Addr, time::Duration};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UdpSocket;
    use tokio::time::timeout;

    use super::*;
    use crate::{
        decode_reliable_udp_data_fragment, encode_reliable_udp_connect,
        encode_reliable_udp_connect_ok, reliable_udp_packet_kind, ControlDelivery, ControlMessage,
        ControlTransport, PingPacket, ReliableUdpConnect, ReliableUdpConnectOk,
        ReliableUdpMulticastMode, ReliableUdpPacketKind,
    };

    #[test]
    fn synchronous_bind_reports_a_missing_tokio_runtime_instead_of_panicking() {
        let error = ReliableUdpSessionHub::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
            .expect_err("binding without an entered runtime must fail");
        assert_eq!(error.kind(), io::ErrorKind::Other);
    }

    fn loopback() -> SocketAddr {
        SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0)
    }

    #[test]
    fn peer_stream_adapter_accepts_cpp_frames_above_the_old_two_mib_cap() {
        const BODY_SIZE: usize = 2 * 1024 * 1024 + 1;

        let (commands, _commands_rx) = mpsc::channel(1);
        let (_inbound_tx, inbound) = mpsc::channel(1);
        let terminal = Arc::new(PeerTerminalState::open());
        let mut stream = ReliableUdpPeerStream::new(
            SocketAddr::from(([127, 0, 0, 1], 11_111)),
            1,
            commands,
            inbound,
            terminal,
        );

        stream.install_read_frame(vec![0x5a; BODY_SIZE]).unwrap();
        assert_eq!(stream.read_frame[0], TCP_FRAME_PREFIX);
        assert_eq!(
            &stream.read_frame[1..TCP_FRAME_HEADER_SIZE],
            &(BODY_SIZE as u32).to_ne_bytes()
        );
        assert_eq!(stream.read_frame.len(), TCP_FRAME_HEADER_SIZE + BODY_SIZE);

        stream.write_buffer.clear();
        stream.write_buffer.push(TCP_FRAME_PREFIX);
        stream
            .write_buffer
            .extend_from_slice(&(BODY_SIZE as u32).to_ne_bytes());
        assert_eq!(
            stream.buffered_frame_size().unwrap(),
            Some(TCP_FRAME_HEADER_SIZE + BODY_SIZE)
        );
    }

    async fn connected_pair() -> (
        ReliableUdpSessionHub,
        ReliableUdpSessionHub,
        ReliableUdpPeerStream,
        ReliableUdpPeerStream,
    ) {
        let outgoing_hub = ReliableUdpSessionHub::bind(loopback()).unwrap();
        let mut incoming_hub = ReliableUdpSessionHub::bind(loopback()).unwrap();
        let (outgoing, incoming) = tokio::join!(
            outgoing_hub.connect(incoming_hub.local_addr()),
            incoming_hub.accept(),
        );
        let outgoing = outgoing.unwrap();
        let incoming = incoming.unwrap();
        assert_eq!(outgoing.peer_addr(), incoming_hub.local_addr());
        assert_eq!(incoming.peer_addr(), outgoing_hub.local_addr());
        (outgoing_hub, incoming_hub, outgoing, incoming)
    }

    #[tokio::test]
    async fn netpuncher_route_reports_own_address_and_punches_without_a_game_peer() {
        async fn write_payload(stream: &mut ReliableUdpPeerStream, payload: &[u8]) {
            let mut frame = Vec::with_capacity(TCP_FRAME_HEADER_SIZE + payload.len());
            frame.push(TCP_FRAME_PREFIX);
            frame.extend_from_slice(&(payload.len() as u32).to_ne_bytes());
            frame.extend_from_slice(payload);
            stream.write_all(&frame).await.unwrap();
            stream.flush().await.unwrap();
        }

        async fn read_payload(stream: &mut ReliableUdpPeerStream) -> Vec<u8> {
            let mut header = [0_u8; TCP_FRAME_HEADER_SIZE];
            stream.read_exact(&mut header).await.unwrap();
            assert_eq!(header[0], TCP_FRAME_PREFIX);
            let length = u32::from_ne_bytes(header[1..].try_into().unwrap()) as usize;
            let mut payload = vec![0_u8; length];
            stream.read_exact(&mut payload).await.unwrap();
            payload
        }

        let mut subject = ReliableUdpSessionHub::bind(loopback()).unwrap();
        let subject_address = subject.local_addr();
        let subject_handle = subject.handle();
        let mut puncher_event_rx = subject.take_puncher_event_receiver();
        let mut puncher = ReliableUdpSessionHub::bind(loopback()).unwrap();
        let puncher_address = puncher.local_addr();

        subject_handle
            .init_puncher(puncher_address, NetpuncherRole::Host)
            .await
            .unwrap();
        let mut puncher_stream = timeout(Duration::from_secs(2), puncher.accept())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(puncher_stream.peer_addr(), subject_address);
        assert_eq!(
            timeout(Duration::from_secs(2), puncher_event_rx.recv())
                .await
                .unwrap(),
            Some(NetpuncherIoEvent::Connected {
                family: NetpuncherAddressFamily::Ipv4,
                puncher_address,
                observed_address: subject_address,
            })
        );
        assert!(timeout(Duration::from_millis(50), subject.accept())
            .await
            .is_err());

        subject_handle
            .send_puncher_packet(
                NetpuncherAddressFamily::Ipv4,
                NetpuncherPacket::IdRequest,
            )
            .await
            .unwrap();
        assert_eq!(
            timeout(Duration::from_secs(2), read_payload(&mut puncher_stream))
                .await
                .unwrap(),
            crate::encode_netpuncher_packet(&NetpuncherPacket::IdRequest)
        );

        let target = UdpSocket::bind(loopback()).await.unwrap();
        let target_address = target.local_addr().unwrap();
        write_payload(
            &mut puncher_stream,
            &crate::encode_netpuncher_packet(&NetpuncherPacket::ClientRequest {
                address: target_address,
            }),
        )
        .await;
        let mut wire = [0_u8; 64];
        let (length, source) = timeout(Duration::from_secs(2), target.recv_from(&mut wire))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(canonical_reliable_udp_peer_address(source), subject_address);
        assert_eq!(length, 9);
        assert_eq!(wire[0], 0x01);
        assert_eq!(&wire[5..9], &0_u32.to_ne_bytes());
        assert!(timeout(Duration::from_millis(50), target.recv_from(&mut wire))
            .await
            .is_err());
        assert!(puncher_event_rx.try_recv().is_err());
        assert!(timeout(Duration::from_millis(50), subject.accept())
            .await
            .is_err());

        let assigned = NetpuncherPacket::AssignId { id: 0x1122_3344 };
        write_payload(
            &mut puncher_stream,
            &crate::encode_netpuncher_packet(&assigned),
        )
        .await;
        assert_eq!(
            timeout(Duration::from_secs(2), puncher_event_rx.recv())
                .await
                .unwrap(),
            Some(NetpuncherIoEvent::Packet {
                family: NetpuncherAddressFamily::Ipv4,
                puncher_address,
                packet: assigned,
            })
        );

        subject_handle
            .close_puncher(puncher_address)
            .await
            .unwrap();
        let mut closed = [0_u8; 1];
        assert_eq!(
            timeout(Duration::from_secs(2), puncher_stream.read(&mut closed))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        drop(puncher_stream);

        subject_handle
            .init_puncher(puncher_address, NetpuncherRole::Host)
            .await
            .unwrap();
        let mut puncher_stream = timeout(Duration::from_secs(2), puncher.accept())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            timeout(Duration::from_secs(2), puncher_event_rx.recv())
                .await
                .unwrap(),
            Some(NetpuncherIoEvent::Connected { .. })
        ));
        write_payload(&mut puncher_stream, &[0x51, 0x02, 0, 0, 0, 0]).await;
        assert_eq!(
            timeout(Duration::from_secs(2), puncher_stream.read(&mut closed))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        assert!(timeout(Duration::from_millis(50), subject.accept())
            .await
            .is_err());

        drop(puncher_stream);
        subject.shutdown().await.unwrap();
        puncher.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn reliable_udp_session_stream_round_trips_control_transport_frames() {
        let (outgoing_hub, incoming_hub, outgoing, incoming) = connected_pair().await;
        let mut outgoing = ControlTransport::new(outgoing);
        let mut incoming = ControlTransport::new(incoming);
        let ping = PingPacket {
            sent_at: 0x1234_5678,
            packet_counter: 9,
        };

        outgoing
            .send_message(ControlMessage::Ping(ping))
            .await
            .unwrap();
        assert_eq!(
            timeout(Duration::from_secs(2), incoming.read_message())
                .await
                .unwrap()
                .unwrap(),
            ControlMessage::Ping(ping)
        );

        let control = clonk_engine::ControlPacket::Message(clonk_engine::MessageControlData {
            message_type: clonk_engine::MESSAGE_TYPE_NORMAL,
            player: 1,
            to_player: -1,
            message: clonk_engine::LegacyCString::from_bytes(vec![b'x'; 1_600]).unwrap(),
            by_client: 1,
        });
        let data = crate::encode_control_entry_payload(&control).unwrap();
        assert!(data.len() > crate::RELIABLE_UDP_DATA_PAYLOAD_LIMIT * 3);
        outgoing
            .send_message(ControlMessage::Packet {
                delivery: ControlDelivery::Direct,
                data: data.clone(),
            })
            .await
            .unwrap();
        assert_eq!(
            timeout(Duration::from_secs(2), incoming.read_message())
                .await
                .unwrap()
                .unwrap(),
            ControlMessage::Packet {
                delivery: ControlDelivery::Direct,
                data,
            }
        );

        incoming
            .send_message(ControlMessage::Pong(ping))
            .await
            .unwrap();
        assert_eq!(
            timeout(Duration::from_secs(2), outgoing.read_message())
                .await
                .unwrap()
                .unwrap(),
            ControlMessage::Pong(ping)
        );

        drop(outgoing);
        drop(incoming);
        outgoing_hub.shutdown().await.unwrap();
        incoming_hub.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn peer_stream_shutdown_sends_close_and_yields_remote_eof() {
        let (outgoing_hub, incoming_hub, mut outgoing, mut incoming) = connected_pair().await;
        outgoing.shutdown().await.unwrap();

        let mut byte = [0; 1];
        assert_eq!(
            timeout(Duration::from_secs(2), incoming.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        assert_eq!(
            incoming
                .write_all(&[TCP_FRAME_PREFIX])
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::BrokenPipe
        );

        drop(outgoing);
        drop(incoming);
        outgoing_hub.shutdown().await.unwrap();
        incoming_hub.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn owned_peer_stream_keeps_its_hub_alive_and_closes_it_on_drop() {
        let outgoing_hub = ReliableUdpSessionHub::bind(loopback()).unwrap();
        let expected_local = outgoing_hub.local_addr();
        let mut incoming_hub = ReliableUdpSessionHub::bind(loopback()).unwrap();
        let incoming_address = incoming_hub.local_addr();
        let (outgoing, incoming) = tokio::join!(
            outgoing_hub.connect_owned(incoming_address),
            incoming_hub.accept(),
        );
        let outgoing = outgoing.unwrap();
        assert_eq!(outgoing.local_addr(), expected_local);
        assert_eq!(outgoing.peer_addr(), incoming_address);
        let mut outgoing = ControlTransport::new(outgoing);
        let mut incoming = ControlTransport::new(incoming.unwrap());
        let ping = PingPacket {
            sent_at: 33,
            packet_counter: 3,
        };

        outgoing
            .send_message(ControlMessage::Ping(ping))
            .await
            .unwrap();
        assert_eq!(
            incoming.read_message().await.unwrap(),
            ControlMessage::Ping(ping)
        );

        drop(outgoing);
        let mut incoming_stream = incoming.into_inner();
        let mut byte = [0; 1];
        assert_eq!(
            timeout(Duration::from_secs(2), incoming_stream.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );

        drop(incoming_stream);
        incoming_hub.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn stale_stream_close_does_not_close_a_reconnected_peer_generation() {
        let mut hub = ReliableUdpSessionHub::bind(loopback()).unwrap();
        let hub_address = hub.local_addr();
        let raw_peer = UdpSocket::bind(loopback()).await.unwrap();
        let raw_peer_address = raw_peer.local_addr().unwrap();
        let mut wire = [0_u8; 512];

        raw_peer
            .send_to(
                &encode_reliable_udp_connect(&ReliableUdpConnect::unicast(0, hub_address)),
                hub_address,
            )
            .await
            .unwrap();
        let (length, source) = timeout(Duration::from_secs(2), raw_peer.recv_from(&mut wire))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(source, hub_address);
        assert_eq!(
            reliable_udp_packet_kind(&wire[..length]),
            Some(ReliableUdpPacketKind::Connect)
        );
        raw_peer
            .send_to(
                &encode_reliable_udp_connect_ok(&ReliableUdpConnectOk {
                    packet_number: 0,
                    multicast_mode: ReliableUdpMulticastMode::NoMulticast,
                    observed_address: hub_address,
                }),
                hub_address,
            )
            .await
            .unwrap();
        let stale_stream = timeout(Duration::from_secs(2), hub.accept())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stale_stream.peer_addr(), raw_peer_address);

        raw_peer
            .send_to(
                &encode_reliable_udp_connect(&ReliableUdpConnect::unicast(17, hub_address)),
                hub_address,
            )
            .await
            .unwrap();
        let replacement_stream = timeout(Duration::from_secs(2), hub.accept())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(replacement_stream.peer_addr(), raw_peer_address);

        // Dropping the disconnected stream queues its Close after the hub has
        // installed the replacement at the same socket address. That stale
        // command must not close the replacement generation.
        drop(stale_stream);
        let ping = PingPacket {
            sent_at: 0x1234_5678,
            packet_counter: 9,
        };
        let mut replacement = ControlTransport::new(replacement_stream);
        replacement
            .send_message(ControlMessage::Ping(ping))
            .await
            .unwrap();

        let mut expected_payload = vec![0x00];
        expected_payload.extend_from_slice(&ping.sent_at.to_ne_bytes());
        expected_payload.extend_from_slice(&ping.packet_counter.to_ne_bytes());
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            let (length, source) = tokio::time::timeout_at(deadline, raw_peer.recv_from(&mut wire))
                .await
                .expect("replacement generation did not send its packet")
                .unwrap();
            assert_eq!(source, hub_address);
            match reliable_udp_packet_kind(&wire[..length]) {
                Some(ReliableUdpPacketKind::Data) => {
                    let fragment = decode_reliable_udp_data_fragment(&wire[..length]).unwrap();
                    assert_eq!(fragment.payload, expected_payload);
                    break;
                }
                Some(ReliableUdpPacketKind::Close) => {
                    panic!("stale stream closed the replacement peer generation")
                }
                _ => {}
            }
        }

        drop(replacement);
        hub.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn saturated_hub_command_queue_backpressures_the_stream_write() {
        let (commands, mut command_rx) = mpsc::channel(1);
        assert!(commands
            .try_send(HubCommand::Shutdown { completion: None })
            .is_ok());
        let (_inbound, inbound_rx) = mpsc::channel(1);
        let terminal = Arc::new(PeerTerminalState::open());
        let mut stream = ReliableUdpPeerStream::new(loopback(), 4, commands, inbound_rx, terminal);
        let mut frame = vec![TCP_FRAME_PREFIX];
        frame.extend_from_slice(&1_u32.to_ne_bytes());
        frame.push(0);

        let mut write = tokio::spawn(async move {
            let result = async {
                stream.write_all(&frame).await?;
                stream.flush().await
            }
            .await;
            (stream, result)
        });
        assert!(timeout(Duration::from_millis(20), &mut write)
            .await
            .is_err());
        assert!(matches!(
            command_rx.try_recv(),
            Ok(HubCommand::Shutdown { .. })
        ));
        let (stream, result) = timeout(Duration::from_secs(2), write)
            .await
            .unwrap()
            .unwrap();
        result.unwrap();
        assert!(matches!(
            command_rx.recv().await,
            Some(HubCommand::Send { payload, .. }) if payload == vec![0]
        ));
        drop(stream);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancelling_shutdown_while_the_command_queue_is_full_releases_the_socket() {
        let hub = ReliableUdpSessionHub::bind(loopback()).unwrap();
        let local_addr = hub.local_addr();
        for generation in 0..HUB_COMMAND_CAPACITY as u64 {
            hub.commands
                .try_send(HubCommand::Close {
                    peer: SocketAddr::from(([127, 0, 0, 1], 30_000)),
                    generation,
                })
                .unwrap();
        }

        let mut shutdown = Box::pin(hub.shutdown());
        let waker = std::task::Waker::noop();
        let mut context = Context::from_waker(waker);
        assert!(matches!(shutdown.as_mut().poll(&mut context), Poll::Pending));
        drop(shutdown);

        let rebound = timeout(Duration::from_secs(2), async {
            loop {
                match ReliableUdpSessionHub::bind(local_addr) {
                    Ok(hub) => break hub,
                    Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
                        tokio::task::yield_now().await;
                    }
                    Err(error) => panic!("unexpected UDP rebind failure: {error}"),
                }
            }
        })
        .await
        .expect("cancelled shutdown leaked the UDP task/socket");
        rebound.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn saturated_peer_inbound_queue_closes_with_a_retained_reason() {
        let mut driver = ReliableUdpSocketDriver::bind(loopback()).unwrap();
        let sink = UdpSocket::bind(loopback()).await.unwrap();
        let peer = sink.local_addr().unwrap();
        driver.connect(peer).await.unwrap();
        let (commands, _command_rx) = mpsc::channel(1);
        let (incoming, _incoming_rx) = mpsc::channel(1);
        let (puncher_events, _puncher_event_rx) = mpsc::channel(PUNCHER_EVENT_CAPACITY);
        let (inbound, inbound_rx) = mpsc::channel(PEER_INBOUND_PACKET_CAPACITY);
        let terminal = Arc::new(PeerTerminalState::open());
        let mut stream =
            ReliableUdpPeerStream::new(peer, 7, commands.clone(), inbound_rx, terminal.clone());
        let mut peers = BTreeMap::from([(
            peer,
            ConnectedPeer {
                generation: 7,
                inbound,
                terminal: terminal.clone(),
            },
        )]);
        for index in 0..PEER_INBOUND_PACKET_CAPACITY {
            assert!(peers[&peer]
                .inbound
                .try_send(PeerInbound::Packet(vec![index as u8]))
                .is_ok());
        }
        let mut pending_connects = BTreeMap::new();
        let mut next_peer_generation = 8;
        dispatch_events(
            &mut driver,
            vec![ReliableUdpEvent::Packet {
                peer,
                payload: vec![0xff],
            }],
            &commands,
            &incoming,
            &puncher_events,
            &mut peers,
            &mut pending_connects,
            &mut next_peer_generation,
        )
        .await;

        assert!(!peers.contains_key(&peer));
        assert!(driver.core().peer_status(peer).is_none());
        assert!(terminal.is_closed());
        assert!(matches!(terminal.reason(), Some(PeerTerminal::Failed(_))));
        for index in 0..PEER_INBOUND_PACKET_CAPACITY {
            let mut frame = [0_u8; TCP_FRAME_HEADER_SIZE + 1];
            stream.read_exact(&mut frame).await.unwrap();
            assert_eq!(frame[0], TCP_FRAME_PREFIX);
            assert_eq!(frame[TCP_FRAME_HEADER_SIZE], index as u8);
        }
        let mut byte = [0_u8; 1];
        let error = stream.read(&mut byte).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::ConnectionAborted);
        assert!(error.to_string().contains("inbound queue is saturated"));
        assert_eq!(
            stream
                .write_all(&[TCP_FRAME_PREFIX])
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::BrokenPipe
        );
    }

    #[tokio::test]
    async fn terminal_signal_closes_writes_before_the_read_side_polls() {
        let (commands, _command_rx) = mpsc::channel(1);
        let (_inbound, inbound_rx) = mpsc::channel(1);
        let terminal = Arc::new(PeerTerminalState::open());
        let mut stream =
            ReliableUdpPeerStream::new(loopback(), 9, commands, inbound_rx, terminal.clone());
        terminal.close(PeerTerminal::Failed("terminal failure".to_string()));

        let error = stream.write_all(&[TCP_FRAME_PREFIX]).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }

    #[tokio::test]
    async fn saturated_incoming_peer_queue_rejects_the_new_peer() {
        let mut driver = ReliableUdpSocketDriver::bind(loopback()).unwrap();
        let sink = UdpSocket::bind(loopback()).await.unwrap();
        let peer = sink.local_addr().unwrap();
        driver.connect(peer).await.unwrap();
        let (commands, _command_rx) = mpsc::channel(1);
        let (incoming, mut incoming_rx) = mpsc::channel(1);
        let (puncher_events, _puncher_event_rx) = mpsc::channel(PUNCHER_EVENT_CAPACITY);
        assert!(incoming
            .try_send(Err(io::Error::new(io::ErrorKind::Other, "occupied")))
            .is_ok());
        let mut peers = BTreeMap::new();
        let mut pending_connects = BTreeMap::new();
        let mut next_peer_generation = 0;

        dispatch_events(
            &mut driver,
            vec![ReliableUdpEvent::Connected {
                peer,
                observed_address: None,
            }],
            &commands,
            &incoming,
            &puncher_events,
            &mut peers,
            &mut pending_connects,
            &mut next_peer_generation,
        )
        .await;

        assert!(peers.is_empty());
        assert!(driver.core().peer_status(peer).is_none());
        assert!(incoming_rx.recv().await.unwrap().is_err());
        assert!(incoming_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn one_hub_accepts_and_keeps_multiple_peer_streams_separate() {
        let first_hub = ReliableUdpSessionHub::bind(loopback()).unwrap();
        let second_hub = ReliableUdpSessionHub::bind(loopback()).unwrap();
        let mut host_hub = ReliableUdpSessionHub::bind(loopback()).unwrap();
        let host_address = host_hub.local_addr();

        let (first, second, accepted) = tokio::join!(
            first_hub.connect(host_address),
            second_hub.connect(host_address),
            async {
                let first = host_hub.accept().await.unwrap();
                let second = host_hub.accept().await.unwrap();
                [first, second]
            },
        );
        let mut first = ControlTransport::new(first.unwrap());
        let mut second = ControlTransport::new(second.unwrap());
        let mut accepted = accepted
            .into_iter()
            .map(|stream| (stream.peer_addr(), ControlTransport::new(stream)))
            .collect::<BTreeMap<_, _>>();
        let first_ping = PingPacket {
            sent_at: 11,
            packet_counter: 1,
        };
        let second_ping = PingPacket {
            sent_at: 22,
            packet_counter: 2,
        };

        first
            .send_message(ControlMessage::Ping(first_ping))
            .await
            .unwrap();
        second
            .send_message(ControlMessage::Ping(second_ping))
            .await
            .unwrap();
        assert_eq!(
            accepted
                .get_mut(&first_hub.local_addr())
                .unwrap()
                .read_message()
                .await
                .unwrap(),
            ControlMessage::Ping(first_ping)
        );
        assert_eq!(
            accepted
                .get_mut(&second_hub.local_addr())
                .unwrap()
                .read_message()
                .await
                .unwrap(),
            ControlMessage::Ping(second_ping)
        );

        drop(first);
        drop(second);
        drop(accepted);
        first_hub.shutdown().await.unwrap();
        second_hub.shutdown().await.unwrap();
        host_hub.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn hub_shutdown_closes_connected_remote_streams() {
        let (outgoing_hub, incoming_hub, outgoing, mut incoming) = connected_pair().await;
        let shutdown = tokio::spawn(outgoing_hub.shutdown());

        let mut byte = [0; 1];
        assert_eq!(
            timeout(Duration::from_secs(2), incoming.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        shutdown.await.unwrap().unwrap();

        drop(outgoing);
        drop(incoming);
        incoming_hub.shutdown().await.unwrap();
    }
}
