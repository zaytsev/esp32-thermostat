use core::{
    array::TryFromSliceError,
    net::{Ipv4Addr, SocketAddrV4},
    ops::Deref,
};

use clatter::{
    Handshaker, NqHandshake,
    bytearray::SensitiveByteArray,
    crypto::{cipher::AesGcm, dh::X25519, hash::Sha256},
    error::{DhError, HandshakeError, TransportError},
    handshakepattern::noise_nn_psk0,
    traits::{Dh, Hash},
    transportstate::TransportState,
};
use defmt::{Format, debug, error, info, trace, warn};
use embassy_futures::select::{Either, Either3, select, select3};
use embassy_net::{
    Stack,
    udp::{PacketMetadata, RecvError, SendError, UdpMetadata, UdpSocket},
};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::{Duration, Instant, Timer};
use esp_hal::rng::Trng;
use esp32_thermostat_common::proto::{
    ClientMessage, DiscoveryRequest, ServerMessage, TelemetryMessage,
};

use crate::AppEventsSender;

const RX_BUF_SIZE: usize = 4096;
const TX_BUF_SIZE: usize = 4096;

const MSG_MAX_LEN: usize = 256;
const NONCE_LEN: usize = 8;

const DEFAULT_PING_INTERVAL: Duration = Duration::from_secs(5);
const DEFAULT_PONG_TIMEOUT: Duration = Duration::from_millis(1500);

const DISCOVERY_PORT: u16 = 40333;
const DISCOVERY_RETRY_TIMEOUT: Duration = Duration::from_secs(5);
const DISCOVERY_RETRY_TIMEOUT_MAX: Duration = Duration::from_secs(120);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);

pub const PRESHARED_KEY_LEN: usize = 32;

pub struct ConnectionOptions {
    pub server_port: u16,
    pub ping_interval: Duration,
    pub pong_timeout: Duration,
    pub server_handshake_timeout: Duration,
    pub discovery_retry_timeout: Duration,
    pub discovery_max_retry_timeout: Duration,
}

impl Default for ConnectionOptions {
    fn default() -> Self {
        Self {
            server_port: DISCOVERY_PORT,
            ping_interval: DEFAULT_PING_INTERVAL,
            pong_timeout: DEFAULT_PONG_TIMEOUT,
            discovery_retry_timeout: DISCOVERY_RETRY_TIMEOUT,
            discovery_max_retry_timeout: DISCOVERY_RETRY_TIMEOUT_MAX,
            server_handshake_timeout: HANDSHAKE_TIMEOUT,
        }
    }
}

#[derive(Clone)]
pub struct PresharedKey {
    value: SensitiveByteArray<[u8; PRESHARED_KEY_LEN]>,
}

impl<'a> From<&'a str> for PresharedKey {
    fn from(value: &'a str) -> Self {
        let value = Sha256::hash(value.as_bytes());
        Self { value }
    }
}

impl Deref for PresharedKey {
    type Target = <SensitiveByteArray<[u8; PRESHARED_KEY_LEN]> as Deref>::Target;

    fn deref(&self) -> &Self::Target {
        self.value.deref()
    }
}

#[derive(Format)]
pub enum ControlMessage {
    SendMetrics {
        inside_temp: f32,
        heater_enabled: bool,
    },
}

pub type ControlChannel =
    embassy_sync::channel::Channel<CriticalSectionRawMutex, ControlMessage, 4>;
// pub type ControlChannelSender<'a> =
//     embassy_sync::channel::Sender<'a, CriticalSectionRawMutex, ControlMessage, 4>;
pub type ControlChannelReceiver<'a> =
    embassy_sync::channel::Receiver<'a, CriticalSectionRawMutex, ControlMessage, 4>;

struct Session {
    remote: UdpMetadata,
    transport: TransportState<AesGcm, Sha256>,
    rx_seq: Option<u64>,
    ping_seq: u64,

    timeout: Duration,
    ping_interval: Duration,
    pong_timeout: Duration,
    waiting_pong: bool,
    recv_start: Option<Instant>,
}

impl Session {
    fn recv_timeout(&mut self) -> Duration {
        self.recv_start = Some(Instant::now());
        self.timeout
    }

    fn adjust_timeout(&mut self) {
        if let Some(recv_start) = self.recv_start {
            let elapsed = Instant::now() - recv_start;
            self.timeout -= elapsed;
        }
    }

    fn next_ping(&mut self) -> ClientMessage {
        let msg = ClientMessage::Ping(self.ping_seq);
        self.ping_seq += 1;
        msg
    }

    fn sent_ping(&mut self) {
        self.waiting_pong = true;
        self.timeout = self.pong_timeout;
    }

    fn recv_pong(&mut self, _id: u64) {
        self.waiting_pong = false;
        self.timeout = self.ping_interval;
    }

    fn write_msg<'a, 'b>(
        &'a mut self,
        msg: &'b ClientMessage,
        output: &'b mut [u8],
        tmp: &'b mut [u8],
    ) -> Result<usize, Error>
    where
        'a: 'b,
    {
        let nonce = self.transport.sending_nonce();
        let nonce = nonce.to_le_bytes();
        output[..NONCE_LEN].copy_from_slice(&nonce);

        let payload = postcard::to_slice(msg, tmp)?;
        let packet_len = self.transport.send(&payload, &mut output[NONCE_LEN..])?;
        let packet_len = packet_len + NONCE_LEN;

        Ok(packet_len)
    }

    fn read_msg(&mut self, input: &[u8], buf: &mut [u8]) -> Result<ServerMessage, Error> {
        let nonce = if input.len() > NONCE_LEN {
            u64::from_le_bytes(input[..NONCE_LEN].try_into()?)
        } else {
            return Err(Error::MessageTooShort);
        };
        self.rx_seq = match self.rx_seq {
            Some(n) if n >= nonce => Err(Error::InvalidPacketSequence),
            _ => Ok(Some(nonce)),
        }?;

        self.transport.set_receiving_nonce(nonce);
        let payload_len = self.transport.receive(&input[NONCE_LEN..], buf)?;
        Ok(postcard::from_bytes::<ServerMessage>(&buf[..payload_len])?)
    }
}

#[derive(defmt::Format)]
enum Error {
    InvalidPacketSequence,
    BufferTooSmall,
    MessageTooShort,
    OneWayViolation,
    CipherError,
    DhError,
    HandshakeFailed,
    NoNetworkRoute,
    InvalidSocketState,
    InvalidFormat(postcard::Error),
    PingTimeout,
}

impl Error {
    fn requires_reconnect(&self) -> bool {
        match self {
            Error::NoNetworkRoute => true,
            _ => false,
        }
    }
}

impl From<TransportError> for Error {
    fn from(value: TransportError) -> Self {
        match value {
            TransportError::BufferTooSmall => Self::BufferTooSmall,
            TransportError::TooShort => Self::MessageTooShort,
            TransportError::OneWayViolation => Self::OneWayViolation,
            TransportError::Cipher(_cipher_error) => Self::CipherError,
        }
    }
}

impl From<postcard::Error> for Error {
    fn from(value: postcard::Error) -> Self {
        Self::InvalidFormat(value)
    }
}

impl From<TryFromSliceError> for Error {
    fn from(_value: TryFromSliceError) -> Self {
        Self::MessageTooShort
    }
}

impl From<DhError> for Error {
    fn from(_value: DhError) -> Self {
        Self::DhError
    }
}

impl From<HandshakeError> for Error {
    fn from(_value: HandshakeError) -> Self {
        Self::HandshakeFailed
    }
}

impl From<SendError> for Error {
    fn from(value: SendError) -> Self {
        match value {
            SendError::NoRoute => Self::NoNetworkRoute,
            SendError::SocketNotBound => Self::InvalidSocketState,
        }
    }
}

impl From<RecvError> for Error {
    fn from(value: RecvError) -> Self {
        match value {
            RecvError::Truncated => Self::BufferTooSmall,
        }
    }
}

#[embassy_executor::task]
pub async fn run(
    sender_id: u64,
    psk: PresharedKey,
    config: ConnectionOptions,
    stack: Stack<'static>,
    mut trng: Trng<'static>,
    control_messages: ControlChannelReceiver<'static>,
    app_events: AppEventsSender<'static>,
) {
    info!("Initializing metrics reporting task");

    let mut rx_meta = [PacketMetadata::EMPTY; 16];
    let mut rx_buf = [0u8; RX_BUF_SIZE];
    let mut tx_meta = [PacketMetadata::EMPTY; 16];
    let mut tx_buf = [0u8; TX_BUF_SIZE];
    let mut socket = UdpSocket::new(stack, &mut rx_meta, &mut rx_buf, &mut tx_meta, &mut tx_buf);
    let _ = socket.bind(0);

    let mut buf = [0u8; MSG_MAX_LEN];
    let mut tmp = [0u8; MSG_MAX_LEN];

    loop {
        let session = match init_session(
            &socket,
            sender_id,
            &*psk,
            &config,
            &mut buf,
            &mut tmp,
            &mut trng,
            &control_messages,
        )
        .await
        {
            Ok(session) => session,
            Err(e) => {
                error!("Failed to open session {:?}", e);
                continue;
            }
        };

        match run_session(
            &socket,
            session,
            &mut buf,
            &mut tmp,
            sender_id,
            &control_messages,
            &app_events,
        )
        .await
        {
            Ok(_) => {}
            Err(_) => {}
        }
    }
}

async fn init_session<'a>(
    socket: &UdpSocket<'a>,
    sender_id: u64,
    psk: &'a [u8],
    opts: &'a ConnectionOptions,
    inp: &'a mut [u8],
    out: &'a mut [u8],
    trng: &'a mut Trng<'_>,
    control_messages: &ControlChannelReceiver<'static>,
) -> Result<Session, Error> {
    let broadcast: UdpMetadata = SocketAddrV4::new(Ipv4Addr::BROADCAST, opts.server_port).into();
    let mut retry_timeout = opts.discovery_retry_timeout;
    let retry_max_timeout = opts.discovery_max_retry_timeout;
    let handshake_timeout = opts.server_handshake_timeout;

    info!("Initializing session");

    loop {
        match select(
            do_handshake(socket, broadcast, sender_id, psk, opts, inp, out, trng),
            Timer::after(handshake_timeout),
        )
        .await
        {
            Either::First(Ok(session)) => {
                break Ok(session);
            }
            Either::First(Err(e)) => {
                error!("Failed to establish session: {:?}", e);
                Timer::after(retry_timeout).await;
            }
            Either::Second(_) => {
                warn!("Timeout receving handshake response");

                if retry_timeout < retry_max_timeout {
                    retry_timeout = retry_timeout.checked_mul(2).unwrap_or(retry_max_timeout);
                }
                if retry_timeout > retry_max_timeout {
                    retry_timeout = retry_max_timeout;
                }
                Timer::after(retry_timeout).await;
            }
        }

        while let Ok(_msg) = control_messages.try_receive() {
            // TODO
        }
    }
}

async fn do_handshake<'a, 's>(
    socket: &UdpSocket<'a>,
    endpoint: UdpMetadata,
    sender_id: u64,
    psk: &'a [u8],
    opts: &'a ConnectionOptions,
    inp: &'a mut [u8],
    out: &'a mut [u8],
    trng: &'a mut Trng<'s>,
) -> Result<Session, Error> {
    debug!("Initializing session handshake");

    let key = X25519::genkey(trng)?;
    let mut handshake = NqHandshake::<X25519, AesGcm, Sha256, _>::new(
        noise_nn_psk0(),
        &[],
        true,
        Some(key),
        None,
        None,
        None,
        trng,
    )?;
    handshake.push_psk(psk);

    debug!("Session handshake initialization complete");

    let payload = DiscoveryRequest { sender_id };
    let message = postcard::to_slice(&payload, inp)?;
    let len = handshake.write_message(message, out)?;

    debug!("Broadcasting handshake packet");

    socket.send_to(&out[..len], endpoint).await?;

    debug!("Waiting for handshake reply");

    let (len, remote) = socket.recv_from(inp).await?;
    debug!("Received handshake response {=usize}", len);
    let _len = handshake.read_message(&inp[..len], out)?;
    let transport = handshake.finalize()?;
    debug!("Transport initialization complete");
    Ok(Session {
        transport,
        remote,
        rx_seq: None,
        ping_seq: 0,
        timeout: opts.ping_interval,
        ping_interval: opts.ping_interval,
        pong_timeout: opts.pong_timeout,
        waiting_pong: false,
        recv_start: None,
    })
}

async fn run_session<'a, 's>(
    socket: &'a UdpSocket<'s>,
    mut session: Session,
    buf: &'a mut [u8],
    tmp: &'a mut [u8],
    sender_id: u64,
    control_messages: &'a ControlChannelReceiver<'static>,
    app_events: &'a AppEventsSender<'static>,
) -> Result<(), Error> {
    loop {
        match select3(
            socket.recv_from(buf),
            control_messages.receive(),
            Timer::after(session.recv_timeout()),
        )
        .await
        {
            Either3::First(Ok((len, _meta))) => match session.read_msg(&buf[..len], tmp) {
                Ok(ServerMessage::StatusQuery) => {
                    session.adjust_timeout();
                }
                Ok(ServerMessage::HeaterParamsUpdate(params)) => {
                    // TODO channel send timeout?
                    app_events
                        .send(crate::AppEvent::UpdateHeaterParams(params))
                        .await;
                    session.adjust_timeout();
                }
                Ok(ServerMessage::Pong(id)) => {
                    trace!("Received pong {=u64}", id);
                    session.recv_pong(id);
                }
                Err(e) => {
                    error!("Failed to read message from server: {:?}", e);
                    session.adjust_timeout();
                }
            },
            Either3::First(Err(e)) => error!("Error receving UDP packet {:?}", e),
            Either3::Second(ctrl) => {
                debug!("Received {:?}", ctrl);
                match ctrl {
                    ControlMessage::SendMetrics {
                        inside_temp,
                        heater_enabled,
                    } => {
                        let msg = ClientMessage::Telemetry(TelemetryMessage {
                            sender_id: sender_id,
                            inside_temp,
                            heater_enabled,
                        });
                        match send_msg(&socket, &mut session, &msg, buf, tmp).await {
                            Ok(_) => {
                                trace!("Sent {:?} message to {:?}", msg, session.remote);
                                session.adjust_timeout();
                            }
                            Err(e) if !e.requires_reconnect() => {
                                error!("Failed to send {:?}: {:?}", msg, e);
                                session.adjust_timeout();
                            }
                            Err(e) => {
                                error!("Unable to send {:?}: {:?}", msg, e);
                            }
                        }
                    }
                }
            }
            Either3::Third(_) if !session.waiting_pong => {
                let ping = session.next_ping();
                match send_msg(&socket, &mut session, &ping, buf, tmp).await {
                    Ok(_) => {
                        trace!("Sent ping request {:?}", ping);
                        session.sent_ping();
                    }
                    Err(e) => {
                        error!("Failed to send ping: {:?}", e);
                        if e.requires_reconnect() {
                            break Err(e);
                        }
                    }
                }
            }
            Either3::Third(_) => {
                warn!("Timeout expired waiting for the PONG response from server");
                break Err(Error::PingTimeout);
            }
        }
    }
}

async fn send_msg<'a, 's>(
    socket: &'a UdpSocket<'s>,
    session: &'a mut Session,
    msg: &ClientMessage,
    buf: &'a mut [u8],
    tmp: &'a mut [u8],
) -> Result<(), Error> {
    let len = session.write_msg(msg, buf, tmp)?;
    socket.send_to(&buf[..len], session.remote).await?;
    Ok(())
}
