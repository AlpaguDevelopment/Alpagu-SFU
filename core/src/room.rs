use crate::{
    protocol::{ClientMessage, MAX_PEERS, PROTOCOL, Pc, Peer, validate_sdp},
    signaling::Command,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::SocketAddr,
    time::{Duration, Instant},
};
use str0m::{
    Candidate, Event, Input, Output, Rtc,
    bwe::Bitrate,
    change::{SdpAnswer, SdpOffer, SdpPendingOffer},
    media::{Direction, KeyframeRequestKind, MediaData, MediaKind, Mid},
    net::{Protocol, Receive},
};
use tokio::sync::mpsc;

const MEDIA_TIMEOUT: Duration = Duration::from_secs(15);

type Result<T> = std::result::Result<T, &'static str>;
pub type Datagram = (SocketAddr, Vec<u8>);

#[derive(Default)]
struct Batch {
    media: Vec<(Mid, MediaKind)>,
    frames: Vec<MediaData>,
    keyframes: Vec<Mid>,
}

struct Connection {
    participant_id: u64,
    leg: Pc,
    rtc: Rtc,
    deadline: Instant,
    media_expires: Option<Instant>,
    answer_expires: Option<Instant>,
}
impl Connection {
    fn new(address: SocketAddr, participant_id: u64, leg: Pc) -> Result<Self> {
        let now = Instant::now();
        // The publishing browser estimates its uplink from our TWCC feedback.
        // Only the subscription transport needs a server-side send estimator.
        let send_estimate = matches!(leg, Pc::Sub).then(|| Bitrate::kbps(1200));
        let mut rtc = Rtc::builder()
            .set_ice_lite(true)
            .clear_codecs()
            .enable_opus(true, false)
            // Opus has no RTX here. Allow modest reordering, then forward the
            // next intact packet for the listener's jitter buffer and FEC.
            .set_reordering_timeout_audio(Some(Duration::from_millis(60)))
            .enable_vp8(true)
            .enable_bwe(send_estimate)
            .set_stats_interval(Some(Duration::from_secs(5)))
            .set_send_buffer_video(1000)
            .build(now);
        rtc.add_local_candidate(Candidate::host(address, "udp").map_err(|_| "Invalid candidate")?)
            .ok_or("Candidate rejected")?;
        Ok(Self {
            participant_id,
            leg,
            rtc,
            deadline: now,
            media_expires: None,
            answer_expires: None,
        })
    }
    fn next_deadline(&self) -> Instant {
        [Some(self.deadline), self.media_expires, self.answer_expires]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(self.deadline)
    }
    fn expired(&self, now: Instant) -> bool {
        [self.media_expires, self.answer_expires]
            .into_iter()
            .flatten()
            .any(|end| now >= end)
    }
    fn drain(&mut self, datagrams: &mut Vec<Datagram>) -> Result<Batch> {
        let mut batch = Batch::default();
        for _ in 0..4096 {
            match self.rtc.poll_output().map_err(|_| "Media output failed")? {
                Output::Timeout(deadline) => {
                    self.deadline = deadline;
                    return Ok(batch);
                }
                Output::Transmit(packet) => {
                    datagrams.push((packet.destination, packet.contents.to_vec()))
                }
                Output::Event(Event::MediaAdded(media)) => {
                    batch.media.push((media.mid, media.kind))
                }
                Output::Event(Event::MediaData(data)) => batch.frames.push(data),
                Output::Event(Event::KeyframeRequest(request)) => batch.keyframes.push(request.mid),
                Output::Event(Event::PeerStats(stats)) if self.rtc.is_connected() => {
                    let rtt = stats.rtt.or_else(|| {
                        stats
                            .selected_candidate_pair
                            .as_ref()?
                            .current_round_trip_time
                    });
                    tracing::info!(
                        participant = self.participant_id, leg = ?self.leg,
                        rx_bytes = stats.peer_bytes_rx, tx_bytes = stats.peer_bytes_tx,
                        rtt_ms = rtt.map(|v| v.as_millis() as u64),
                        rx_loss_pct = stats.ingress_loss_fraction.map(|v| v * 100.0),
                        tx_loss_pct = stats.egress_loss_fraction.map(|v| v * 100.0),
                        estimated_tx_bps = stats.bwe_tx.map(|v| v.as_u64()),
                        "Transport quality"
                    );
                }
                _ => {}
            }
        }
        Err("Media output limit exceeded")
    }
}

struct Slot {
    mid: Mid,
    source: Option<u64>,
    kind: MediaKind,
    awaiting_keyframe: bool,
}
struct Participant {
    peer: Peer,
    moderator: bool,
    tx: mpsc::Sender<Value>,
    publisher: Connection,
    subscriber: Connection,
    published: bool,
    pub_restart: Option<SdpPendingOffer>,
    restart_sub: bool,
    audio: Option<Mid>,
    video: Option<Mid>,
    request_keyframe: bool,
    last_keyframe_request: Option<Instant>,
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
    fn subscriptions(&mut self, sources: &[(u64, MediaKind)], now: Instant) {
        if self.pending.is_some() {
            return;
        }
        // Retire departed sources in a separate negotiation before assigning their slots.
        let mut retiring = Vec::new();
        for slot in &mut self.slots {
            if slot
                .source
                .is_some_and(|id| !sources.contains(&(id, slot.kind)))
            {
                slot.source = None;
                slot.awaiting_keyframe = slot.kind == MediaKind::Video;
                retiring.push(slot.mid);
                // Drop cached packets and use fresh SSRCs so old-source RTX cannot leak
                // into the next publication. Keep the m-line reusable (not stopped).
                let mut direct = self.subscriber.rtc.direct_api();
                if let Some(previous) = direct.stream_tx_by_mid(slot.mid, None).map(|tx| tx.ssrc())
                {
                    // str0m allocates fresh SSRCs when the reused MID becomes sendonly.
                    direct.remove_stream_tx(previous);
                }
            }
        }
        let mut changes = self.subscriber.rtc.sdp_api();
        for &mid in &retiring {
            changes.set_direction(mid, Direction::Inactive);
        }
        if retiring.is_empty() {
            for &(id, kind) in sources {
                if self
                    .slots
                    .iter()
                    .any(|s| s.source == Some(id) && s.kind == kind)
                {
                    continue;
                }
                if let Some(slot) = self
                    .slots
                    .iter_mut()
                    .find(|s| s.source.is_none() && s.kind == kind)
                {
                    slot.source = Some(id);
                    slot.awaiting_keyframe = kind == MediaKind::Video;
                    changes.set_direction(slot.mid, Direction::SendOnly);
                } else {
                    let mid = changes.add_media(kind, Direction::SendOnly, None, None, None);
                    self.slots.push(Slot {
                        mid,
                        source: Some(id),
                        kind,
                        awaiting_keyframe: kind == MediaKind::Video,
                    });
                }
            }
        }
        if std::mem::take(&mut self.restart_sub) && !self.slots.is_empty() {
            changes.ice_restart(true);
        }
        if let Some((offer, pending)) = changes.apply() {
            self.subscriber
                .media_expires
                .get_or_insert(now + MEDIA_TIMEOUT);
            self.subscriber.answer_expires = Some(now + MEDIA_TIMEOUT);
            self.pending = Some(pending);
            let tracks: Vec<_> = self
                .slots
                .iter()
                .filter_map(|slot| {
                    slot.source.map(|source| {
                        json!({"mid":slot.mid.to_string(), "peer":source.to_string(),
                    "kind":if slot.kind == MediaKind::Audio { "audio" } else { "video" }})
                    })
                })
                .collect();
            self.send(json!({"type":"sub_offer", "sdp":offer.to_sdp_string(), "tracks":tracks}));
        }
    }
    fn drain_subscriber(&mut self, datagrams: &mut Vec<Datagram>, requests: &mut BTreeSet<u64>) {
        match self.subscriber.drain(datagrams) {
            Ok(batch) => {
                for mid in batch.keyframes {
                    if let Some(source) = self
                        .slots
                        .iter()
                        .find(|s| s.mid == mid && s.kind == MediaKind::Video)
                        .and_then(|s| s.source)
                    {
                        requests.insert(source);
                    }
                }
            }
            Err(_) => self.failed = true,
        }
    }
}

pub struct Room {
    peers: BTreeMap<u64, Participant>,
    moderator_key: Option<String>,
    address: SocketAddr,
    now: Instant,
}
impl Room {
    pub fn new(address: SocketAddr, moderator_key: Option<String>) -> Self {
        Self {
            moderator_key,
            peers: BTreeMap::new(),
            address,
            now: Instant::now(),
        }
    }
    pub fn deadline(&self) -> Instant {
        self.peers
            .values()
            .flat_map(|p| [p.publisher.next_deadline(), p.subscriber.next_deadline()])
            .min()
            .unwrap_or_else(|| Instant::now() + Duration::from_secs(1))
    }
    pub fn command(&mut self, command: Command) {
        self.now = self.now.max(Instant::now());
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
        let name = name.trim().to_owned();
        if self.peers.values().any(|p| p.peer.name == name) {
            return Err("Name is already in use");
        }
        if self.peers.len() >= MAX_PEERS {
            return Err("Room is full");
        }
        let peer = Peer {
            id: id.to_string(),
            name,
            mic: false,
            cam: false,
            deaf: false,
            server_muted: false,
        };
        let mut p = Participant {
            peer: peer.clone(),
            moderator: false,
            tx,
            publisher: Connection::new(self.address, id, Pc::Pub)?,
            subscriber: Connection::new(self.address, id, Pc::Sub)?,
            published: false,
            pub_restart: None,
            restart_sub: false,
            audio: None,
            video: None,
            request_keyframe: false,
            last_keyframe_request: None,
            slots: Vec::new(),
            pending: None,
            failed: false,
        };
        p.publisher.media_expires = Some(self.now + MEDIA_TIMEOUT);
        let peers: Vec<_> = self.peers.values().map(|p| &p.peer).collect();
        p.send(json!({"type":"joined", "id":peer.id, "protocol":PROTOCOL, "server":env!("CARGO_PKG_VERSION"), "peers":peers}));
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
            ClientMessage::RestartIce { pc: Pc::Pub } => {
                if !p.published {
                    return Err("Publication has not been negotiated");
                }
                if p.pub_restart.is_none() {
                    let mut changes = p.publisher.rtc.sdp_api();
                    changes.ice_restart(true);
                    let (offer, pending) =
                        changes.apply().ok_or("Cannot restart publication ICE")?;
                    p.pub_restart = Some(pending);
                    p.publisher.answer_expires = Some(self.now + MEDIA_TIMEOUT);
                    p.send(json!({"type":"pub_restart_offer", "sdp":offer.to_sdp_string()}));
                }
            }
            ClientMessage::RestartIce { pc: Pc::Sub } => {
                p.restart_sub = true;
            }
            ClientMessage::PubRestartAnswer { sdp } => {
                validate_sdp(&sdp, true)?;
                let pending = p
                    .pub_restart
                    .take()
                    .ok_or("Unexpected publication restart answer")?;
                let answer = SdpAnswer::from_sdp_string(&sdp)
                    .map_err(|_| "Invalid publication restart SDP")?;
                p.publisher
                    .rtc
                    .sdp_api()
                    .accept_answer(pending, answer)
                    .map_err(|_| "Publication restart failed")?;
                p.publisher.answer_expires = None;
                p.request_keyframe = true;
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
                p.subscriber.answer_expires = None;
            }
            ClientMessage::State { mic, cam, deaf } => {
                p.peer.mic = mic;
                if cam && !p.peer.cam {
                    p.request_keyframe = true;
                }
                p.peer.cam = cam;
                p.peer.deaf = deaf;
                self.broadcast(id, json!({"type":"peer_state", "id":id.to_string(), "mic":mic, "cam":cam, "deaf":deaf}));
            }
            ClientMessage::ClaimModerator { key } => {
                if self.moderator_key.as_ref() != Some(&key) {
                    return Err("Moderator access denied");
                }
                p.moderator = true;
                p.send(json!({"type":"moderator_granted"}));
            }
            ClientMessage::SetServerMute { id: target, muted } => {
                if !p.moderator {
                    return Err("Moderator access required");
                }
                let target_id = target
                    .parse::<u64>()
                    .map_err(|_| "Invalid participant ID")?;
                let Some(target_peer) = self.peers.get_mut(&target_id) else {
                    if let Some(actor) = self.peers.get_mut(&id) {
                        actor.send(
                            json!({"type":"error","message":"Participant has left","fatal":false}),
                        );
                    }
                    return Ok(());
                };
                target_peer.peer.server_muted = muted;
                let notice = json!({"type":"server_mute","id":target_peer.peer.id,"muted":muted});
                target_peer.send(notice.clone());
                self.broadcast(target_id, notice);
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
        self.now = now;
        let Ok(packet) = Receive::new(Protocol::Udp, source, self.address, bytes) else {
            return;
        };
        let input = Input::Receive(now, packet);
        for p in self.peers.values_mut() {
            for connection in [&mut p.publisher, &mut p.subscriber] {
                if connection.rtc.accepts(&input) {
                    if connection.rtc.handle_input(input).is_err() {
                        p.failed = true;
                    } else if connection.rtc.is_connected() {
                        connection.media_expires = Some(now + MEDIA_TIMEOUT);
                    }
                    return;
                }
            }
        }
    }
    pub fn timeout(&mut self, now: Instant) {
        self.now = now;
        for p in self.peers.values_mut() {
            for (leg, c) in [("pub", &mut p.publisher), ("sub", &mut p.subscriber)] {
                if c.expired(now) || !c.rtc.is_alive() {
                    tracing::info!(transport = leg, "Media transport closed or timed out");
                    p.failed = true;
                    break;
                }
                if c.deadline <= now && c.rtc.handle_input(Input::Timeout(now)).is_err() {
                    p.failed = true;
                }
            }
        }
    }
    pub fn pump(&mut self) -> Vec<Datagram> {
        let mut datagrams = Vec::new();
        let mut frames = Vec::new();
        let mut requests = BTreeSet::new();
        for (&id, p) in &mut self.peers {
            match p.publisher.drain(&mut datagrams) {
                Ok(batch) => {
                    for (mid, kind) in batch.media {
                        match kind {
                            MediaKind::Audio => p.audio = Some(mid),
                            MediaKind::Video => p.video = Some(mid),
                        }
                    }
                    for data in batch.frames {
                        let kind = if p.audio == Some(data.mid) {
                            MediaKind::Audio
                        } else if p.video == Some(data.mid) {
                            MediaKind::Video
                        } else {
                            continue;
                        };
                        // Bound the size of frames retained during fanout.
                        if kind == MediaKind::Audio && (p.peer.server_muted || p.peer.deaf) {
                            continue;
                        }
                        if data.data.len() <= 512 * 1024 {
                            frames.push((id, kind, data));
                        } else if kind == MediaKind::Video {
                            requests.insert(id);
                        }
                    }
                }
                Err(_) => p.failed = true,
            }
            p.drain_subscriber(&mut datagrams, &mut requests);
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
            .flat_map(|(&id, p)| {
                [(p.audio, MediaKind::Audio), (p.video, MediaKind::Video)]
                    .into_iter()
                    .filter_map(move |(mid, kind)| mid.map(|_| (id, kind)))
            })
            .collect();
        for (&id, p) in &mut self.peers {
            p.subscriptions(
                &sources
                    .iter()
                    .copied()
                    .filter(|(source, _)| *source != id)
                    .collect::<Vec<_>>(),
                self.now,
            );
            p.drain_subscriber(&mut datagrams, &mut requests);
            if p.pending.is_none() && p.subscriber.rtc.is_connected() {
                for slot in &p.slots {
                    if slot.awaiting_keyframe
                        && let Some(source) = slot.source
                    {
                        requests.insert(source);
                    }
                }
            }
        }
        for (source, kind, data) in frames {
            if !self.peers.contains_key(&source) {
                continue;
            }
            for p in self.peers.values_mut() {
                if p.pending.is_some() || p.failed || !p.subscriber.rtc.is_connected() {
                    continue;
                }
                if kind == MediaKind::Audio && p.peer.deaf {
                    continue;
                }
                let Some(slot) = p
                    .slots
                    .iter_mut()
                    .find(|s| s.source == Some(source) && s.kind == kind)
                else {
                    continue;
                };
                if kind == MediaKind::Video {
                    if !data.contiguous {
                        slot.awaiting_keyframe = true;
                    }
                    if slot.awaiting_keyframe && !data.is_keyframe() {
                        requests.insert(source);
                        continue;
                    }
                }
                if let Some(writer) = p.subscriber.rtc.writer(slot.mid)
                    && let Some(pt) = writer.match_params(data.params)
                {
                    if writer
                        .write(pt, data.network_time, data.time, data.data.clone())
                        .is_ok()
                    {
                        slot.awaiting_keyframe = false;
                    } else if kind == MediaKind::Video {
                        slot.awaiting_keyframe = true;
                        requests.insert(source);
                    }
                }
                // Drain after every write; this also handles downstream recovery feedback.
                p.drain_subscriber(&mut datagrams, &mut requests);
            }
        }
        for (&id, p) in &mut self.peers {
            p.request_keyframe |= requests.contains(&id);
            if !p.request_keyframe
                || !p.publisher.rtc.is_connected()
                || p.last_keyframe_request.is_some_and(|last| {
                    self.now.saturating_duration_since(last) < Duration::from_millis(500)
                })
            {
                continue;
            }
            let Some(mut writer) = p.video.and_then(|mid| p.publisher.rtc.writer(mid)) else {
                continue;
            };
            let kind = if writer.is_request_keyframe_possible(KeyframeRequestKind::Pli) {
                KeyframeRequestKind::Pli
            } else {
                KeyframeRequestKind::Fir
            };
            // Before the first RTP packet the receiving SSRC may be unknown; retry later.
            if writer.request_keyframe(None, kind).is_ok() {
                p.request_keyframe = false;
                p.last_keyframe_request = Some(self.now);
            }
            if p.publisher.drain(&mut datagrams).is_err() {
                p.failed = true;
            }
        }
        datagrams
    }
}
#[cfg(test)]
mod tests;
