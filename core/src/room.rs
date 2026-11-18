use crate::{
    protocol::{ClientMessage, MAX_PEERS, Peer, validate_sdp},
    signaling::Command,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    time::{Duration, Instant},
};
use str0m::{
    Candidate, Event, Input, Output, Rtc,
    change::{SdpAnswer, SdpOffer, SdpPendingOffer},
    media::{Direction, MediaData, MediaKind, Mid},
    net::{Protocol, Receive},
};
use tokio::sync::mpsc;

type Result<T> = std::result::Result<T, &'static str>;
pub type Datagram = (SocketAddr, Vec<u8>);

struct Connection {
    rtc: Rtc,
    deadline: Instant,
}
impl Connection {
    fn new(address: SocketAddr) -> Result<Self> {
        let now = Instant::now();
        let mut rtc = Rtc::builder()
            .set_ice_lite(true)
            .clear_codecs()
            .enable_opus(true, false)
            .build(now);
        rtc.add_local_candidate(Candidate::host(address, "udp").map_err(|_| "Invalid candidate")?)
            .ok_or("Candidate rejected")?;
        Ok(Self { rtc, deadline: now })
    }
    fn drain(&mut self, datagrams: &mut Vec<Datagram>) -> Result<(Option<Mid>, Vec<MediaData>)> {
        let mut audio = None;
        let mut frames = Vec::new();
        for _ in 0..4096 {
            match self.rtc.poll_output().map_err(|_| "Media output failed")? {
                Output::Timeout(deadline) => {
                    self.deadline = deadline;
                    return Ok((audio, frames));
                }
                Output::Transmit(packet) => {
                    datagrams.push((packet.destination, packet.contents.to_vec()))
                }
                Output::Event(Event::MediaAdded(media)) if media.kind == MediaKind::Audio => {
                    audio = Some(media.mid)
                }
                Output::Event(Event::MediaData(data)) => frames.push(data),
                _ => {}
            }
        }
        Err("Media output limit exceeded")
    }
}

struct Slot {
    mid: Mid,
    source: Option<u64>,
}
struct Participant {
    peer: Peer,
    tx: mpsc::Sender<Value>,
    publisher: Connection,
    subscriber: Connection,
    published: bool,
    audio: Option<Mid>,
    slots: Vec<Slot>,
    pending: Option<SdpPendingOffer>,
    failed: bool,
}
impl Participant {
    fn send(&mut self, value: Value) {
        if self.tx.try_send(value).is_err() {
            self.failed = true;
        }
    }
    fn subscriptions(&mut self, sources: &[u64]) {
        if self.pending.is_some() {
            return;
        }
        let mut changes = self.subscriber.rtc.sdp_api();
        for slot in &mut self.slots {
            if slot.source.is_some_and(|id| !sources.contains(&id)) {
                slot.source = None;
                changes.set_direction(slot.mid, Direction::Inactive);
            }
        }
        for &id in sources {
            if self.slots.iter().any(|slot| slot.source == Some(id)) {
                continue;
            }
            if let Some(slot) = self.slots.iter_mut().find(|slot| slot.source.is_none()) {
                slot.source = Some(id);
                changes.set_direction(slot.mid, Direction::SendOnly);
            } else {
                let mid =
                    changes.add_media(MediaKind::Audio, Direction::SendOnly, None, None, None);
                self.slots.push(Slot {
                    mid,
                    source: Some(id),
                });
            }
        }
        if let Some((offer, pending)) = changes.apply() {
            self.pending = Some(pending);
            let tracks: Vec<_> = self
                .slots
                .iter()
                .filter_map(|slot| {
                    slot.source.map(|source|
                json!({"mid":slot.mid.to_string(), "peer":source.to_string(), "kind":"audio"}))
                })
                .collect();
            self.send(json!({"type":"sub_offer", "sdp":offer.to_sdp_string(), "tracks":tracks}));
        }
    }
}

pub struct Room {
    peers: BTreeMap<u64, Participant>,
    address: SocketAddr,
}
impl Room {
    pub fn new(address: SocketAddr) -> Self {
        Self {
            peers: BTreeMap::new(),
            address,
        }
    }
    pub fn deadline(&self) -> Instant {
        self.peers
            .values()
            .flat_map(|p| [p.publisher.deadline, p.subscriber.deadline])
            .min()
            .unwrap_or_else(|| Instant::now() + Duration::from_secs(1))
    }
    pub fn command(&mut self, command: Command) {
        match command {
            Command::Join { id, name, tx } => {
                if let Err(message) = self.join(id, name, tx.clone()) {
                    let _ = tx.try_send(json!({"type":"error", "message":message, "fatal":true}));
                }
            }
            Command::Message { id, message } => {
                if let Err(message) = self.message(id, message) {
                    if let Some(p) = self.peers.get_mut(&id) {
                        p.send(json!({"type":"error", "message":message, "fatal":true}));
                    }
                    self.remove(id);
                }
            }
            Command::Left(id) => self.remove(id),
        }
    }
    fn join(&mut self, id: u64, name: String, tx: mpsc::Sender<Value>) -> Result<()> {
        if self.peers.len() >= MAX_PEERS {
            return Err("Room is full");
        }
        let peer = Peer {
            id: id.to_string(),
            name,
            mic: false,
            cam: false,
            deaf: false,
        };
        let mut p = Participant {
            peer: peer.clone(),
            tx,
            publisher: Connection::new(self.address)?,
            subscriber: Connection::new(self.address)?,
            published: false,
            audio: None,
            slots: Vec::new(),
            pending: None,
            failed: false,
        };
        let peers: Vec<_> = self.peers.values().map(|p| &p.peer).collect();
        p.send(json!({"type":"joined", "id":peer.id, "protocol":1, "server":env!("CARGO_PKG_VERSION"), "peers":peers}));
        self.broadcast(id, json!({"type":"peer_joined", "peer":peer}));
        self.peers.insert(id, p);
        tracing::info!(participants = self.peers.len(), "Participant joined");
        Ok(())
    }
    fn broadcast(&mut self, except: u64, value: Value) {
        for (&id, p) in &mut self.peers {
            if id != except {
                p.send(value.clone());
            }
        }
    }
    fn remove(&mut self, id: u64) {
        if self.peers.remove(&id).is_some() {
            self.broadcast(id, json!({"type":"peer_left", "id":id.to_string()}));
            tracing::info!(participants = self.peers.len(), "Participant left");
        }
    }
    fn message(&mut self, id: u64, message: ClientMessage) -> Result<()> {
        let Some(p) = self.peers.get_mut(&id) else {
            return Ok(());
        };
        match message {
            ClientMessage::PubOffer { sdp } => {
                if p.published {
                    return Err("Publish negotiation already completed");
                }
                validate_sdp(&sdp, true)?;
                let offer = SdpOffer::from_sdp_string(&sdp).map_err(|_| "Invalid publish SDP")?;
                let answer = p
                    .publisher
                    .rtc
                    .sdp_api()
                    .accept_offer(offer)
                    .map_err(|_| "Publish negotiation failed")?;
                p.published = true;
                p.send(json!({"type":"pub_answer", "sdp":answer.to_sdp_string()}));
            }
            ClientMessage::SubAnswer { sdp } => {
                validate_sdp(&sdp, false)?;
                let pending = p.pending.take().ok_or("Unexpected subscription answer")?;
                let answer =
                    SdpAnswer::from_sdp_string(&sdp).map_err(|_| "Invalid subscription SDP")?;
                p.subscriber
                    .rtc
                    .sdp_api()
                    .accept_answer(pending, answer)
                    .map_err(|_| "Subscription negotiation failed")?;
            }
            ClientMessage::State { mic, cam, deaf } => {
                p.peer.mic = mic;
                p.peer.cam = cam;
                p.peer.deaf = deaf;
                self.broadcast(id, json!({"type":"peer_state", "id":id.to_string(), "mic":mic, "cam":cam, "deaf":deaf}));
            }
            ClientMessage::Ping {} => p.send(json!({"type":"pong"})),
            ClientMessage::Ice {
                pc,
                candidate,
                sdp_mid,
                sdp_m_line_index,
            } => {
                // ICE-lite learns the remote address from STUN; trickled candidates are optional.
                let _ = (pc, candidate, sdp_mid, sdp_m_line_index);
            }
            ClientMessage::Leave {} => self.remove(id),
            ClientMessage::Join { .. } => return Err("Already joined"),
        }
        Ok(())
    }
    pub fn receive(&mut self, source: SocketAddr, bytes: &[u8], now: Instant) {
        let Ok(packet) = Receive::new(Protocol::Udp, source, self.address, bytes) else {
            return;
        };
        let input = Input::Receive(now, packet);
        for p in self.peers.values_mut() {
            for connection in [&mut p.publisher, &mut p.subscriber] {
                if connection.rtc.accepts(&input) {
                    if connection.rtc.handle_input(input).is_err() {
                        p.failed = true;
                    }
                    return;
                }
            }
        }
    }
    pub fn timeout(&mut self, now: Instant) {
        for p in self.peers.values_mut() {
            for c in [&mut p.publisher, &mut p.subscriber] {
                if c.deadline <= now && c.rtc.handle_input(Input::Timeout(now)).is_err() {
                    p.failed = true;
                }
            }
        }
    }
    pub fn pump(&mut self) -> Vec<Datagram> {
        let mut datagrams = Vec::new();
        let mut frames = Vec::new();
        for (&id, p) in &mut self.peers {
            match p.publisher.drain(&mut datagrams) {
                Ok((audio, incoming)) => {
                    if let Some(mid) = audio {
                        p.audio = Some(mid);
                    }
                    frames.extend(
                        incoming
                            .into_iter()
                            .filter(|data| p.audio == Some(data.mid))
                            .map(|data| (id, data)),
                    );
                }
                Err(_) => p.failed = true,
            }
            if p.subscriber.drain(&mut datagrams).is_err() {
                p.failed = true;
            }
        }
        let failed: Vec<_> = self
            .peers
            .iter()
            .filter(|(_, p)| p.failed || p.tx.is_closed())
            .map(|(&id, _)| id)
            .collect();
        for id in failed {
            self.remove(id);
        }
        let sources: Vec<_> = self
            .peers
            .iter()
            .filter(|(_, p)| p.audio.is_some())
            .map(|(&id, _)| id)
            .collect();
        for (&id, p) in &mut self.peers {
            p.subscriptions(
                &sources
                    .iter()
                    .copied()
                    .filter(|source| *source != id)
                    .collect::<Vec<_>>(),
            );
            if p.subscriber.drain(&mut datagrams).is_err() {
                p.failed = true;
            }
        }
        for (source, data) in frames {
            for p in self.peers.values_mut() {
                if p.pending.is_some() || p.failed {
                    continue;
                }
                let Some(mid) = p
                    .slots
                    .iter()
                    .find(|slot| slot.source == Some(source))
                    .map(|slot| slot.mid)
                else {
                    continue;
                };
                if let Some(writer) = p.subscriber.rtc.writer(mid)
                    && let Some(pt) = writer.match_params(data.params)
                {
                    // Preserve the publisher's media clock and map its codec to the subscriber's PT.
                    if writer
                        .write(pt, data.network_time, data.time, data.data.clone())
                        .is_err()
                    {
                        tracing::debug!("Audio frame could not be forwarded");
                    }
                }
                // str0m requires draining after every write, including within one input batch.
                if p.subscriber.drain(&mut datagrams).is_err() {
                    p.failed = true;
                }
            }
        }
        datagrams
    }
}
#[cfg(test)]
mod tests;
