use std::{net::SocketAddr, sync::Arc, time::Duration};

use anyhow::{Context, anyhow};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use clatter::{
    Handshaker, NqHandshake,
    crypto::{cipher::AesGcm, dh::X25519, hash::Sha256},
    handshakepattern::noise_nn_psk0,
    traits::Dh,
    transportstate::TransportState,
};
use dashmap::{
    DashMap,
    mapref::{multiple::RefMulti, one::Ref as EntryRef},
};
use esp32_thermostat_common::proto::{
    ClientMessage, DiscoveryRequest, HeaterParams, ServerMessage, TelemetryMessage,
};
use rand::{SeedableRng, rngs::StdRng};
use tokio::{
    net::{ToSocketAddrs, UdpSocket},
    sync::{mpsc, oneshot},
    task,
    time::{self, Instant},
};
use tracing::{debug, error, trace, warn};

type ClientTransport = TransportState<AesGcm, Sha256>;

struct Session {
    node_id: u64,
    transport: ClientTransport,
    initiated: Instant,
    last_active: Instant,
    rx_counter: u64,
    tx_counter: u64,
    err_counter: u64,
    rx_nonce: Option<u64>,

    created_at: DateTime<Utc>,
}

impl Session {
    fn init<'a, 'b>(
        message: &'a [u8],
        psk: &'b [u8],
        rng: &'b mut StdRng,
    ) -> anyhow::Result<(Self, Vec<u8>)> {
        let key = X25519::genkey(rng)?;
        let mut handshake = NqHandshake::<X25519, AesGcm, Sha256, _>::new(
            noise_nn_psk0(),
            &[],
            false,
            Some(key),
            None,
            None,
            None,
            rng,
        )?;
        handshake.push_psk(psk);
        debug!("Initialized session handshake state");

        debug!("Incoming message length={}", message.len());

        let mut input_buf = [0u8; 1024];
        let len = handshake.read_message(message, input_buf.as_mut_slice())?;
        debug!("Read handshake payload of size {}", len);
        let node_id = if len > 0 {
            match postcard::from_bytes::<DiscoveryRequest>(&input_buf[..len]) {
                Ok(discovery_request) => Ok(discovery_request.sender_id),
                Err(e) => Err(anyhow!("Invalid client handshake payload: {}", e)),
            }
        } else {
            Err(anyhow!("Empty client handshake"))
        }?;
        let mut output_buf = vec![0u8; 1024];
        let len = handshake.write_message(&[], output_buf.as_mut_slice())?;
        debug!("Written handshake payload of size {}", len);
        output_buf.truncate(len);

        let transport = handshake.finalize()?;
        let now = Instant::now();

        Ok((
            Self {
                node_id,
                transport,
                initiated: now,
                last_active: now,
                rx_counter: 0,
                tx_counter: 0,
                err_counter: 0,
                rx_nonce: None,
                created_at: Utc::now(),
            },
            output_buf,
        ))
    }

    fn check_received_nonce(&self, nonce: u64) -> bool {
        match self.rx_nonce {
            Some(n) if nonce <= n => false,
            _ => true,
        }
    }

    fn message_received(&mut self, nonce: u64) {
        self.last_active = Instant::now();
        self.rx_counter += 1;
        self.rx_nonce = Some(nonce);
    }

    fn message_sent(&mut self) {
        self.tx_counter += 1;
    }

    fn message_err(&mut self) {
        self.err_counter += 1;
    }
}

const NONCE_LEN: usize = 8;
const MIN_PACKET_LENGTH: usize = NONCE_LEN + 1;
// const PSK: &[u8] = include_bytes!("../../../key.bin");
//

#[derive(Debug)]
enum NodeCommand {
    SendUpdateHeaterParams {
        node_id: u64,
        params: HeaterParams,
        reply: oneshot::Sender<()>,
    },
    ListNodes(oneshot::Sender<Vec<NodeInfo>>),
    FindNode {
        node_id: u64,
        reply: oneshot::Sender<Option<NodeInfo>>,
    },
    SendPingReply {
        addr: SocketAddr,
        payload: u64,
    },
}

#[derive(Clone)]
pub struct NodeManager {
    tx: mpsc::Sender<NodeCommand>,
}

impl NodeManager {
    pub async fn set_target_temp(&self, node_id: u64, temp: f32, tolerance: f32) {
        let (tx, rx) = oneshot::channel();
        // TODO: handle channel errors
        let _ = self
            .tx
            .send(NodeCommand::SendUpdateHeaterParams {
                node_id: node_id,
                params: HeaterParams {
                    target_temp: temp,
                    temp_tolerance: tolerance,
                },
                reply: tx,
            })
            .await;
        let _ = rx.await;
    }

    pub async fn list_nodes(&self) -> Vec<NodeInfo> {
        let (tx, rx) = oneshot::channel();
        // TODO: handle channel errors
        let _ = self.tx.send(NodeCommand::ListNodes(tx)).await;
        rx.await.unwrap_or_else(|_| Vec::new())
    }

    pub async fn find_node_by_id(&self, node_id: u64) -> Option<NodeInfo> {
        let (tx, rx) = oneshot::channel();
        // TODO: handle channel errors
        let _ = self
            .tx
            .send(NodeCommand::FindNode { node_id, reply: tx })
            .await;
        rx.await.unwrap_or_default()
    }
}

#[async_trait]
pub trait TelemetryProcessor {
    async fn process(&self, msg: TelemetryMessage);
}

pub struct Server {
    sessions: DashMap<SocketAddr, Session>,
    node_ids: DashMap<u64, SocketAddr>,
    psk: Vec<u8>,
    cmd_rx: mpsc::Receiver<NodeCommand>,
    node_manager: NodeManager,
    processor: Arc<dyn TelemetryProcessor + Send + Sync>,
}

impl Server {
    pub fn new<P>(shared_secret: &str, telemetry_processor: P) -> Self
    where
        P: TelemetryProcessor + Send + Sync + 'static,
    {
        let psk = <sha2::Sha256 as sha2::Digest>::digest(&shared_secret.as_bytes()).to_vec();
        let (cmd_tx, cmd_rx) = mpsc::channel::<NodeCommand>(128);
        let node_manager = NodeManager { tx: cmd_tx };
        Self {
            sessions: DashMap::new(),
            node_ids: DashMap::new(),
            psk,
            cmd_rx,
            node_manager,
            processor: Arc::new(telemetry_processor),
        }
    }

    pub async fn serve<A>(self, addr: A) -> anyhow::Result<()>
    where
        A: ToSocketAddrs,
    {
        let socket = UdpSocket::bind(addr).await?;
        let rng = StdRng::from_entropy();
        self.run(socket, rng).await
    }

    pub fn node_manager(&self) -> NodeManager {
        self.node_manager.clone()
    }

    async fn run(mut self, socket: UdpSocket, mut rng: StdRng) -> anyhow::Result<()> {
        let mut gc_interval = time::interval(Duration::from_secs(5));
        let max_inactive = Duration::from_secs(30);
        loop {
            let mut buf = [0u8; 1024];
            tokio::select! {
                res = socket.recv_from(&mut buf) => match res {
                    Ok(res) => match self.handle_request(res, &mut buf, &socket, &mut rng).await {
                        Ok(_) => {
                            debug!("processed request");
                        },
                        Err(e) => {
                            warn!("Error processing request: {:?}", e);
                        },
                    },
                    Err(e) => {
                        warn!("Error receving data from socket: {}", e);
                    },
                },
                cmd = self.cmd_rx.recv() => {
                    match cmd {
                        Some(cmd) => match self.handle_command(cmd, &socket).await {
                            Ok(_) => {
                                debug!("");
                            },
                            Err(e) => {
                                warn!("Error processing command: {}", e);
                            },
                        },
                        None => {
                            error!("Command channel is closed!")
                        },
                    };
                },
                _ = gc_interval.tick() => {
                    self.remove_inactive_sessions(max_inactive);
                },
            }
        }
    }

    fn remove_inactive_sessions(&self, inactivity_timeout: Duration) {
        let expired: Vec<_> = self
            .sessions
            .iter()
            .filter_map(|session| {
                if session.last_active.elapsed() > inactivity_timeout {
                    Some((session.key().clone(), session.node_id))
                } else {
                    None
                }
            })
            .collect();

        for (addr, node_id) in expired {
            if self.sessions.remove(&addr).is_some() {
                self.node_ids.remove(&node_id);
                debug!("Removed inactive session {}", addr);
            }
        }
    }

    async fn handle_command(&self, cmd: NodeCommand, socket: &UdpSocket) -> anyhow::Result<()> {
        match cmd {
            NodeCommand::SendUpdateHeaterParams {
                node_id,
                params,
                reply,
            } => {
                if let Some(addr) = self.node_ids.get(&node_id) {
                    if let Some(mut session) = self.sessions.get_mut(&addr) {
                        self.send_message(
                            socket,
                            session.pair_mut(),
                            &ServerMessage::HeaterParamsUpdate(params),
                        )
                        .await?;
                    }
                }
                let _ = reply.send(());
            }
            NodeCommand::ListNodes(reply) => {
                let _ = reply.send(self.get_nodes());
            }
            NodeCommand::SendPingReply { addr, payload } => {
                if let Some(mut entry) = self.sessions.get_mut(&addr) {
                    self.send_message(socket, entry.pair_mut(), &ServerMessage::Pong(payload))
                        .await?;
                }
            }
            NodeCommand::FindNode { node_id, reply } => {
                let result = self
                    .node_ids
                    .get(&node_id)
                    .and_then(|e| self.sessions.get(e.value()))
                    .map(NodeInfo::from);
                let _ = reply.send(result);
            }
        };
        Ok(())
    }

    async fn send_message(
        &self,
        socket: &UdpSocket,
        (node_addr, session): (&SocketAddr, &mut Session),
        msg: &ServerMessage,
    ) -> anyhow::Result<()> {
        let mut buf = [0u8; 1024];
        let packet_len = {
            let (nonce, input) = buf.split_at_mut(NONCE_LEN);
            nonce.copy_from_slice(&session.transport.sending_nonce().to_le_bytes());
            let payload_len = { postcard::to_slice(msg, input)?.len() };
            session.transport.send_in_place(input, payload_len)? + NONCE_LEN
        };
        socket.send_to(&buf[..packet_len], node_addr).await?;
        session.message_sent();

        Ok(())
    }

    async fn handle_request(
        &self,
        (len, src): (usize, SocketAddr),
        buf: &mut [u8],
        socket: &UdpSocket,
        rng: &mut StdRng,
    ) -> anyhow::Result<()> {
        match self.sessions.get_mut(&src) {
            Some(mut entry) if len > MIN_PACKET_LENGTH => {
                let (src, session) = entry.pair_mut();
                match self.handle_message(src, session, len, buf).await {
                    Ok(_) => Ok(()),
                    Err(e) => {
                        session.err_counter += 1;
                        Err(e)
                    }
                }
            }
            Some(_) => Err(anyhow!("Message payload is to short: {}", len)),
            None => {
                debug!("No session found for {}, creating new", src);
                let (session, payload) =
                    Session::init(&buf[..len], &self.psk, rng).context("initializing session")?;

                debug!("Sending handhshake respone to {}", src);
                let len = socket
                    .send_to(&payload, src)
                    .await
                    .context("sending handshake response")?;

                debug!("Handshake response {} sent. Saving session", len);
                self.node_ids.insert(session.node_id, src);
                self.sessions.insert(src, session);

                Ok(())
            }
        }
    }

    async fn handle_message(
        &self,
        from: &SocketAddr,
        session: &mut Session,
        len: usize,
        buf: &mut [u8],
    ) -> Result<(), anyhow::Error> {
        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(&buf[..NONCE_LEN]);
        let nonce = u64::from_le_bytes(nonce);
        debug!(
            "Received packet len={} from {} with nonce={}",
            len, from, nonce
        );
        if !session.check_received_nonce(nonce) {
            return Err(anyhow!("Invalid nonce received: {}", nonce));
        }
        session.transport.set_receiving_nonce(nonce);
        trace!("Decoding message payload");
        let mut payload = [0u8; 1024];
        let message = &buf[NONCE_LEN..len];
        let len = session
            .transport
            .receive(message, &mut payload)
            .context("processing protocol message")?;
        let payload = &payload[..len];
        trace!("Deserializing message payload");
        let payload = postcard::from_bytes::<ClientMessage>(&payload)
            .context("deserializing client message")?;
        session.message_received(nonce);

        match payload {
            ClientMessage::Telemetry(telemetry) => {
                debug!("Received telemetry from {}: {:?}", from, telemetry);
                let processor = self.processor.clone();
                task::spawn(async move { processor.process(telemetry).await });
            }
            ClientMessage::Ping(n) => {
                debug!("Received ping request from client {}", from);
                self.node_manager
                    .tx
                    .send(NodeCommand::SendPingReply {
                        addr: from.clone(),
                        payload: n,
                    })
                    .await?;
            }
        }
        Ok(())
    }

    fn get_nodes(&self) -> Vec<NodeInfo> {
        self.sessions.iter().map(NodeInfo::from).collect()
    }
}

#[derive(Debug, Clone)]
pub struct NodeInfo {
    pub addr: SocketAddr,
    pub node_id: u64,
    pub connected: Instant,
    pub last_active: Instant,
    pub msgs_rx: u64,
    pub msgs_tx: u64,
    pub errors: u64,

    pub connected_at: DateTime<Utc>,
}

impl NodeInfo {
    #[inline(always)]
    fn from_session((addr, session): (&SocketAddr, &Session)) -> NodeInfo {
        NodeInfo {
            addr: *addr,
            node_id: session.node_id,
            connected: session.initiated,
            connected_at: session.created_at,
            last_active: session.last_active,
            msgs_rx: session.rx_counter,
            msgs_tx: session.tx_counter,
            errors: session.err_counter,
        }
    }
}

impl<'a> From<RefMulti<'a, SocketAddr, Session>> for NodeInfo {
    fn from(entry: RefMulti<'a, SocketAddr, Session>) -> Self {
        NodeInfo::from_session(entry.pair())
    }
}

impl<'a> From<EntryRef<'a, SocketAddr, Session>> for NodeInfo {
    fn from(entry: EntryRef<'a, SocketAddr, Session>) -> Self {
        NodeInfo::from_session(entry.pair())
    }
}
