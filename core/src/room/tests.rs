use super::*;
use std::{collections::VecDeque, sync::Once};
use str0m::media::MediaTime;

fn crypto() {
    static INIT: Once = Once::new();
    INIT.call_once(|| str0m::crypto::from_feature_flags().install_process_default());
}
fn test_room() -> Room {
    crypto();
    Room::new("192.0.2.1:7882".parse().unwrap())
}
fn join(room: &mut Room, id: u64) -> mpsc::Receiver<Value> {
    let (tx, rx) = mpsc::channel(128);
    room.command(Command::Join {
        id,
        name: format!("Peer {id}"),
        tx,
    });
    rx
}

#[test]
fn membership_state_capacity_and_cleanup() {
    let mut room = test_room();
    let mut receivers = Vec::new();
    for id in 1..=16 {
        receivers.push(join(&mut room, id));
    }
    let mut rejected = join(&mut room, 17);
    assert_eq!(rejected.try_recv().unwrap()["fatal"], true);
    assert_eq!(room.peers.len(), 16);
    assert_eq!(receivers[0].try_recv().unwrap()["type"], "joined");
    while receivers[0].try_recv().is_ok() {}
    room.command(Command::Message {
        id: 2,
        message: ClientMessage::State {
            mic: true,
            cam: false,
            deaf: true,
        },
    });
    let state = receivers[0].try_recv().unwrap();
    assert_eq!(state["type"], "peer_state");
    assert_eq!(state["deaf"], true);
    for id in 1..=16 {
        room.command(Command::Left(id));
    }
    assert!(room.peers.is_empty());
    for id in 20..30 {
        let _rx = join(&mut room, id);
        room.command(Command::Left(id));
        assert!(room.peers.is_empty());
    }
}

struct Client {
    id: u64,
    publisher: Rtc,
    subscriber: Rtc,
    pub_addr: SocketAddr,
    sub_addr: SocketAddr,
    pending: Option<SdpPendingOffer>,
    mid: Mid,
    video_mid: Mid,
    video_tick: u64,
    keyframe_requests: usize,
    rx: mpsc::Receiver<Value>,
    received: Vec<(Mid, Vec<u8>)>,
    mapping: Vec<Value>,
    max_media_lines: usize,
}
impl Client {
    fn new(room: &mut Room, id: u64, now: Instant) -> Self {
        let rx = join(room, id);
        let pub_addr = SocketAddr::from(([192, 0, 2, 2], 10000 + id as u16 * 2));
        let sub_addr = SocketAddr::from(([192, 0, 2, 2], 10001 + id as u16 * 2));
        let mut publisher = Rtc::builder().build(now);
        publisher
            .add_local_candidate(Candidate::host(pub_addr, "udp").unwrap())
            .unwrap();
        let mut changes = publisher.sdp_api();
        let mid = changes.add_media(MediaKind::Audio, Direction::SendOnly, None, None, None);
        let video_mid = changes.add_media(MediaKind::Video, Direction::SendOnly, None, None, None);
        let (offer, pending) = changes.apply().unwrap();
        room.command(Command::Message {
            id,
            message: ClientMessage::PubOffer {
                sdp: offer.to_sdp_string(),
            },
        });
        let mut subscriber = Rtc::builder().build(now);
        subscriber
            .add_local_candidate(Candidate::host(sub_addr, "udp").unwrap())
            .unwrap();
        Self {
            id,
            publisher,
            subscriber,
            pub_addr,
            sub_addr,
            pending: Some(pending),
            mid,
            video_mid,
            video_tick: 0,
            keyframe_requests: 0,
            rx,
            received: Vec::new(),
            mapping: Vec::new(),
            max_media_lines: 0,
        }
    }
    fn signal(&mut self, room: &mut Room) -> bool {
        let mut changed = false;
        while let Ok(value) = self.rx.try_recv() {
            changed = true;
            match value["type"].as_str().unwrap() {
                "pub_answer" => {
                    let answer =
                        SdpAnswer::from_sdp_string(value["sdp"].as_str().unwrap()).unwrap();
                    self.publisher
                        .sdp_api()
                        .accept_answer(self.pending.take().unwrap(), answer)
                        .unwrap();
                }
                "pub_restart_offer" => {
                    let offer = SdpOffer::from_sdp_string(value["sdp"].as_str().unwrap()).unwrap();
                    let answer = self.publisher.sdp_api().accept_offer(offer).unwrap();
                    room.message(
                        self.id,
                        ClientMessage::PubRestartAnswer {
                            sdp: answer.to_sdp_string(),
                        },
                    )
                    .unwrap();
                }
                "sub_offer" => {
                    self.max_media_lines = self.max_media_lines.max(
                        value["sdp"]
                            .as_str()
                            .unwrap()
                            .lines()
                            .filter(|line| line.starts_with("m="))
                            .count(),
                    );
                    self.mapping = value["tracks"].as_array().unwrap().clone();
                    let offer = SdpOffer::from_sdp_string(value["sdp"].as_str().unwrap()).unwrap();
                    let answer = self.subscriber.sdp_api().accept_offer(offer).unwrap();
                    room.command(Command::Message {
                        id: self.id,
                        message: ClientMessage::SubAnswer {
                            sdp: answer.to_sdp_string(),
                        },
                    });
                }
                "error" => panic!("Server error: {value}"),
                _ => {}
            }
        }
        changed
    }
    fn drain(&mut self, queue: &mut VecDeque<(SocketAddr, SocketAddr, Vec<u8>)>) {
        for (rtc, addr) in [
            (&mut self.publisher, self.pub_addr),
            (&mut self.subscriber, self.sub_addr),
        ] {
            loop {
                match rtc.poll_output().unwrap() {
                    Output::Timeout(_) => break,
                    Output::Transmit(packet) => {
                        queue.push_back((addr, packet.destination, packet.contents.to_vec()))
                    }
                    Output::Event(Event::MediaData(data)) => {
                        self.received.push((data.mid, data.data.to_vec()))
                    }
                    Output::Event(Event::KeyframeRequest(_)) => self.keyframe_requests += 1,
                    _ => {}
                }
            }
        }
    }
    fn write_audio(&mut self, now: Instant, tick: u64) {
        let writer = self.publisher.writer(self.mid).unwrap();
        let pt = writer
            .payload_params()
            .find(|p| p.spec().codec == str0m::format::Codec::Opus)
            .unwrap()
            .pt();
        // A valid Opus comfort-noise frame; forwarding must preserve its encoded bytes.
        writer
            .write(
                pt,
                now,
                MediaTime::from(Duration::from_millis(tick * 20)),
                vec![0xf8, 0xff, 0xfe],
            )
            .unwrap();
    }
    fn write_video(&mut self, now: Instant, keyframe: bool) -> Vec<u8> {
        let writer = self.publisher.writer(self.video_mid).unwrap();
        let pt = writer
            .payload_params()
            .find(|p| p.spec().codec == str0m::format::Codec::Vp8)
            .unwrap()
            .pt();
        // Synthetic encoded-frame fixture: enough header for VP8 packetization/keyframe detection.
        // This tests transport integrity, not decoding or visual quality.
        let mut frame = vec![self.id as u8; 3000];
        frame[..10].copy_from_slice(&[
            if keyframe { 0x10 } else { 0x11 },
            0,
            0,
            0x9d,
            1,
            0x2a,
            0x80,
            2,
            0xe0,
            1,
        ]);
        self.video_tick += 1;
        writer
            .write(
                pt,
                now,
                MediaTime::from_millis(self.video_tick * 33),
                frame.clone(),
            )
            .unwrap();
        frame
    }
    fn request_video_keyframe(&mut self, source: u64, kind: KeyframeRequestKind) {
        let source = source.to_string();
        let mid = self
            .mapping
            .iter()
            .find(|t| t["peer"].as_str() == Some(source.as_str()) && t["kind"] == "video")
            .unwrap()["mid"]
            .as_str()
            .unwrap();
        self.subscriber
            .writer(Mid::from(mid))
            .unwrap()
            .request_keyframe(None, kind)
            .unwrap();
    }
}
#[derive(Default)]
struct Network {
    offline: Option<SocketAddr>,
    // Local endpoint -> public endpoint after a simulated NAT rebinding.
    rebound: Vec<(SocketAddr, SocketAddr)>,
    loss: Option<(SocketAddr, SocketAddr, u8, usize)>,
    dropped: bool,
    rtx: usize,
}
fn exchange(room: &mut Room, clients: &mut [Client], now: Instant) {
    exchange_network(room, clients, now, &mut Network::default());
}
fn exchange_network(room: &mut Room, clients: &mut [Client], now: Instant, network: &mut Network) {
    let mut queue = VecDeque::new();
    for _ in 0..200 {
        let mut signal = false;
        for client in clients.iter_mut() {
            signal |= client.signal(room);
            client.drain(&mut queue);
        }
        for (destination, bytes) in room.pump() {
            queue.push_back((room.address, destination, bytes));
        }
        if queue.is_empty() && !signal {
            return;
        }
        while let Some((mut source, mut destination, bytes)) = queue.pop_front() {
            if destination == room.address {
                if let Some((_, public)) =
                    network.rebound.iter().find(|(local, _)| *local == source)
                {
                    source = *public;
                }
            } else if let Some((local, _)) = network
                .rebound
                .iter()
                .find(|(_, public)| *public == destination)
            {
                destination = *local;
            } else if network
                .rebound
                .iter()
                .any(|(local, _)| *local == destination)
            {
                continue;
            }
            if network.offline == Some(source) || network.offline == Some(destination) {
                continue;
            }
            let rtp = bytes.len() >= 12 && bytes[0] >> 6 == 2;
            // The synthetic clients negotiate the library's default VP8/RTX PTs (96/97).
            if rtp && bytes[1] & 0x7f == 97 {
                network.rtx += 1;
            }
            if let Some((from, to, pt, skip)) = &mut network.loss
                && source == *from
                && destination == *to
                && rtp
                && bytes[1] & 0x7f == *pt
            {
                if *skip > 0 {
                    *skip -= 1;
                } else {
                    network.loss = None;
                    network.dropped = true;
                    continue;
                }
            }
            if destination == room.address {
                room.receive(source, &bytes, now);
                for (destination, bytes) in room.pump() {
                    queue.push_back((room.address, destination, bytes));
                }
            } else {
                let client = clients
                    .iter_mut()
                    .find(|c| c.pub_addr == destination || c.sub_addr == destination)
                    .unwrap();
                let rtc = if client.pub_addr == destination {
                    &mut client.publisher
                } else {
                    &mut client.subscriber
                };
                rtc.handle_input(Input::Receive(
                    now,
                    Receive::new(Protocol::Udp, source, destination, &bytes).unwrap(),
                ))
                .unwrap();
                client.drain(&mut queue);
            }
        }
    }
    panic!("Exchange did not settle");
}
fn advance(room: &mut Room, clients: &mut [Client], now: &mut Instant, ticks: u64, audio: bool) {
    advance_network(room, clients, now, ticks, audio, &mut Network::default());
}
fn advance_network(
    room: &mut Room,
    clients: &mut [Client],
    now: &mut Instant,
    ticks: u64,
    audio: bool,
    network: &mut Network,
) {
    for tick in 0..ticks {
        *now += Duration::from_millis(20);
        room.timeout(*now);
        for client in clients.iter_mut() {
            client.publisher.handle_input(Input::Timeout(*now)).unwrap();
            client
                .subscriber
                .handle_input(Input::Timeout(*now))
                .unwrap();
        }
        exchange_network(room, clients, *now, network);
        if audio {
            for client in clients.iter_mut() {
                client.write_audio(*now, tick);
            }
            exchange_network(room, clients, *now, network);
        }
    }
}

#[test]
fn forwards_opus_between_three_peers_and_removes_departed_tracks() {
    let mut room = test_room();
    let mut now = Instant::now();
    let mut clients: Vec<_> = (1..=3).map(|id| Client::new(&mut room, id, now)).collect();
    exchange(&mut room, &mut clients, now);
    advance(&mut room, &mut clients, &mut now, 100, false);
    advance(&mut room, &mut clients, &mut now, 30, true);
    for client in &clients {
        assert_eq!(client.mapping.len(), 4);
        assert!(
            !client.received.is_empty(),
            "Peer {} received no audio",
            client.id
        );
        for track in client.mapping.iter().filter(|t| t["kind"] == "audio") {
            assert!(client.received.iter().any(|(mid, data)| mid.to_string()
                == track["mid"].as_str().unwrap()
                && data == &[0xf8, 0xff, 0xfe]));
        }
    }
    clients.pop();
    room.command(Command::Left(3));
    exchange(&mut room, &mut clients, now);
    for client in &clients {
        assert_eq!(client.mapping.len(), 2);
    }
    clients.push(Client::new(&mut room, 4, now));
    exchange(&mut room, &mut clients, now);
    advance(&mut room, &mut clients, &mut now, 100, false);
    for client in &mut clients {
        client.received.clear();
    }
    advance(&mut room, &mut clients, &mut now, 30, true);
    for client in &clients {
        assert_eq!(client.mapping.len(), 4);
        assert!(!client.received.is_empty());
        assert!(client.mapping.iter().all(|t| t["peer"] != "3"));
    }
    assert!(room.peers.values().all(|p| p.slots.len() == 4));
}

#[test]
fn defers_subscription_changes_until_answer() {
    let mut room = test_room();
    let mut rx = join(&mut room, 1);
    rx.try_recv().unwrap();
    let peer = room.peers.get_mut(&1).unwrap();
    peer.subscriptions(&[(2, MediaKind::Audio)], room.now);
    let offer = rx.try_recv().unwrap();
    peer.subscriptions(&[(2, MediaKind::Audio), (3, MediaKind::Audio)], room.now);
    assert!(rx.try_recv().is_err());
    assert_eq!(peer.slots.len(), 1);
    let mut client = Rtc::new(Instant::now());
    let answer = client
        .sdp_api()
        .accept_offer(SdpOffer::from_sdp_string(offer["sdp"].as_str().unwrap()).unwrap())
        .unwrap();
    room.message(
        1,
        ClientMessage::SubAnswer {
            sdp: answer.to_sdp_string(),
        },
    )
    .unwrap();
    room.peers
        .get_mut(&1)
        .unwrap()
        .subscriptions(&[(2, MediaKind::Audio), (3, MediaKind::Audio)], room.now);
    assert_eq!(
        rx.try_recv().unwrap()["tracks"].as_array().unwrap().len(),
        2
    );
}

fn video_room() -> (Room, Vec<Client>, Instant) {
    let mut room = test_room();
    let mut now = Instant::now();
    let mut clients: Vec<_> = (1..=3).map(|id| Client::new(&mut room, id, now)).collect();
    exchange(&mut room, &mut clients, now);
    advance(&mut room, &mut clients, &mut now, 100, false);
    for client in &mut clients {
        client.write_video(now, true);
    }
    exchange(&mut room, &mut clients, now);
    advance(&mut room, &mut clients, &mut now, 40, false);
    (room, clients, now)
}

#[test]
fn forwards_vp8_and_routes_pli_and_fir_to_the_correct_publisher() {
    let (mut room, mut clients, mut now) = video_room();
    for client in &clients {
        for track in client.mapping.iter().filter(|t| t["kind"] == "video") {
            let source: u8 = track["peer"].as_str().unwrap().parse().unwrap();
            assert!(client.received.iter().any(|(mid, data)| mid.to_string()
                == track["mid"].as_str().unwrap()
                && data.last() == Some(&source)));
        }
    }
    for kind in [KeyframeRequestKind::Pli, KeyframeRequestKind::Fir] {
        for c in &mut clients {
            c.keyframe_requests = 0;
        }
        clients[1].request_video_keyframe(1, kind);
        exchange(&mut room, &mut clients, now);
        advance(&mut room, &mut clients, &mut now, 30, false);
        assert!(clients[0].keyframe_requests > 0);
        assert_eq!(clients[1].keyframe_requests, 0);
        assert_eq!(clients[2].keyframe_requests, 0);
    }
}

#[test]
fn new_subscriber_waits_for_keyframe_and_recycled_slots_do_not_mix_publishers() {
    let (mut room, mut clients, mut now) = video_room();
    let original_mid = clients[0]
        .mapping
        .iter()
        .find(|t| t["peer"] == "3" && t["kind"] == "video")
        .unwrap()["mid"]
        .clone();
    let video_mid: Mid = original_mid.as_str().unwrap().into();
    let old_ssrc = room
        .peers
        .get_mut(&1)
        .unwrap()
        .subscriber
        .rtc
        .direct_api()
        .stream_tx_by_mid(video_mid, None)
        .unwrap()
        .ssrc();
    clients.pop();
    room.command(Command::Left(3));
    exchange(&mut room, &mut clients, now);
    clients.push(Client::new(&mut room, 4, now));
    exchange(&mut room, &mut clients, now);
    advance(&mut room, &mut clients, &mut now, 100, false);
    for c in &mut clients {
        c.received.clear();
        c.keyframe_requests = 0;
    }
    // A new publisher cannot feed dependent frames into a recycled decoder.
    let delta = clients[2].write_video(now, false);
    exchange(&mut room, &mut clients, now);
    advance(&mut room, &mut clients, &mut now, 40, false);
    assert!(!clients[0].received.iter().any(|(_, data)| *data == delta));
    assert!(clients[2].keyframe_requests > 0);
    let keyframe = clients[2].write_video(now, true);
    exchange(&mut room, &mut clients, now);
    advance(&mut room, &mut clients, &mut now, 20, false);
    let new_mid = clients[0]
        .mapping
        .iter()
        .find(|t| t["peer"] == "4" && t["kind"] == "video")
        .unwrap()["mid"]
        .clone();
    assert_eq!(original_mid, new_mid);
    let new_ssrc = room
        .peers
        .get_mut(&1)
        .unwrap()
        .subscriber
        .rtc
        .direct_api()
        .stream_tx_by_mid(video_mid, None)
        .unwrap()
        .ssrc();
    assert_ne!(old_ssrc, new_ssrc);

    assert!(
        clients[0]
            .received
            .iter()
            .any(|(mid, data)| mid.to_string() == new_mid.as_str().unwrap() && *data == keyframe)
    );
    assert!(
        clients[0]
            .received
            .iter()
            .all(|(_, data)| data.last() != Some(&3))
    );
    // Existing viewers retain a bounded SDP while new peers repeatedly replace one another.
    for id in 5..15 {
        let departed = clients.pop().unwrap().id;
        room.command(Command::Left(departed));
        clients.push(Client::new(&mut room, id, now));
        exchange(&mut room, &mut clients, now);
        advance(&mut room, &mut clients, &mut now, 30, false);
        assert_eq!(clients[0].mapping.len(), 4);
        assert_eq!(clients[0].max_media_lines, 4);
    }
}

#[test]
fn nack_rtx_recovers_a_lost_video_packet_on_each_transport_leg() {
    let (mut room, mut clients, mut now) = video_room();
    for upstream in [true, false] {
        let (source, destination) = if upstream {
            (clients[0].pub_addr, room.address)
        } else {
            (room.address, clients[1].sub_addr)
        };
        let mut network = Network {
            loss: Some((source, destination, 96, 1)),
            ..Network::default()
        };
        clients[1].received.clear();
        let frame = clients[0].write_video(now, true);
        exchange_network(&mut room, &mut clients, now, &mut network);
        for _ in 0..60 {
            now += Duration::from_millis(20);
            room.timeout(now);
            for c in &mut clients {
                c.publisher.handle_input(Input::Timeout(now)).unwrap();
                c.subscriber.handle_input(Input::Timeout(now)).unwrap();
            }
            exchange_network(&mut room, &mut clients, now, &mut network);
        }
        assert!(network.dropped, "Loss fixture did not drop a packet");
        assert!(network.rtx > 0, "No RTX observed for upstream={upstream}");
        assert!(
            clients[1].received.iter().any(|(_, data)| *data == frame),
            "Frame was not recovered for upstream={upstream}"
        );
    }
}

#[test]
fn camera_restart_requests_are_coalesced_and_rate_limited() {
    let (mut room, mut clients, mut now) = video_room();
    clients[0].keyframe_requests = 0;
    for _ in 0..10 {
        for cam in [false, true] {
            room.command(Command::Message {
                id: 1,
                message: ClientMessage::State {
                    mic: true,
                    cam,
                    deaf: false,
                },
            });
        }
    }
    exchange(&mut room, &mut clients, now);
    advance(&mut room, &mut clients, &mut now, 5, false);
    assert!(clients[0].keyframe_requests <= 1);
    advance(&mut room, &mut clients, &mut now, 30, false);
    assert_eq!(clients[0].keyframe_requests, 1);
}

#[test]
fn full_room_negotiates_thirty_subscription_lines_within_sdp_limits() {
    let mut room = test_room();
    let mut now = Instant::now();
    let mut clients: Vec<_> = (1..=16).map(|id| Client::new(&mut room, id, now)).collect();
    // Certificate generation can take longer than the simulated handshake window.
    now = Instant::now();
    exchange(&mut room, &mut clients, now);
    advance(&mut room, &mut clients, &mut now, 100, false);
    assert_eq!(room.peers.len(), 16);
    for client in &clients {
        assert_eq!(client.mapping.len(), 30);
        assert_eq!(client.max_media_lines, 30);
        assert!(client.subscriber.is_connected());
    }
}

#[test]
fn duplicate_name_preserves_original_and_releases_after_timeout() {
    let mut room = test_room();
    let mut first = join(&mut room, 1);
    first.try_recv().unwrap();
    let (tx, mut duplicate) = mpsc::channel(8);
    room.command(Command::Join {
        id: 2,
        name: " Peer 1 ".into(),
        tx,
    });
    assert_eq!(
        duplicate.try_recv().unwrap()["message"],
        "Name is already in use"
    );
    assert!(first.try_recv().is_err());
    assert_eq!(room.peers.len(), 1);
    let end = room.peers[&1].publisher.media_expires.unwrap();
    room.timeout(end - Duration::from_millis(1));
    room.pump();
    assert!(room.peers.contains_key(&1));
    room.timeout(end);
    room.pump();
    assert!(room.peers.is_empty());
    let (tx, mut fresh) = mpsc::channel(8);
    room.command(Command::Join {
        id: 3,
        name: "Peer 1".into(),
        tx,
    });
    room.command(Command::Left(1));
    assert_eq!(fresh.try_recv().unwrap()["id"], "3");
    assert!(room.peers.contains_key(&3));
}

#[test]
fn ice_restart_preserves_transports_membership_and_forwarding() {
    let (mut room, mut clients, mut now) = video_room();
    let before = clients[0].mapping.clone();
    let old_pub = clients[0].publisher.direct_api().local_ice_credentials();
    let old_sub = clients[0].subscriber.direct_api().local_ice_credentials();
    let mut network = Network {
        rebound: vec![
            (clients[0].pub_addr, "192.0.2.44:22000".parse().unwrap()),
            (clients[0].sub_addr, "192.0.2.44:22001".parse().unwrap()),
        ],
        ..Network::default()
    };
    for pc in [Pc::Pub, Pc::Sub] {
        room.message(1, ClientMessage::RestartIce { pc }).unwrap();
    }
    advance_network(&mut room, &mut clients, &mut now, 100, false, &mut network);
    assert_ne!(
        clients[0].publisher.direct_api().local_ice_credentials(),
        old_pub
    );
    assert_ne!(
        clients[0].subscriber.direct_api().local_ice_credentials(),
        old_sub
    );
    assert_eq!(clients[0].mapping, before);
    for c in &mut clients {
        c.received.clear();
    }
    advance_network(&mut room, &mut clients, &mut now, 30, true, &mut network);
    for (receiver, source) in [(0, "2"), (1, "1")] {
        let mid = clients[receiver]
            .mapping
            .iter()
            .find(|t| t["peer"] == source && t["kind"] == "audio")
            .unwrap()["mid"]
            .as_str()
            .unwrap();
        assert!(
            clients[receiver]
                .received
                .iter()
                .any(|(m, data)| m.to_string() == mid && data == &[0xf8, 0xff, 0xfe])
        );
    }
}

#[test]
fn losing_either_media_leg_expires_only_the_disconnected_participant() {
    for lost_pub in [true, false] {
        let (mut room, mut clients, mut now) = video_room();
        let lost = if lost_pub {
            clients[0].pub_addr
        } else {
            clients[0].sub_addr
        };
        let mut net = Network {
            offline: Some(lost),
            ..Network::default()
        };
        let start = now;
        while now < start + MEDIA_TIMEOUT {
            now += Duration::from_millis(100);
            room.timeout(now);
            for client in &mut clients {
                client.publisher.handle_input(Input::Timeout(now)).unwrap();
                client.subscriber.handle_input(Input::Timeout(now)).unwrap();
            }
            exchange_network(&mut room, &mut clients, now, &mut net);
        }
        assert!(!room.peers.contains_key(&1));
        assert_eq!(room.peers.len(), 2);
        assert!(clients[1].mapping.iter().all(|t| t["peer"] != "1"));
    }
}

#[test]
fn lone_connected_publisher_survives_without_subscriber_traffic() {
    let mut room = test_room();
    let mut now = Instant::now();
    let mut clients = [Client::new(&mut room, 1, now)];
    advance(&mut room, &mut clients, &mut now, 1100, false);
    assert!(room.peers[&1].publisher.rtc.is_connected());
    assert!(room.peers[&1].subscriber.media_expires.is_none());
}

#[test]
fn pending_subscription_answer_expires_despite_healthy_media() {
    let (mut room, mut clients, mut now) = video_room();
    room.message(1, ClientMessage::RestartIce { pc: Pc::Sub })
        .unwrap();
    room.pump();
    let offer = clients[0].rx.try_recv().unwrap();
    assert_eq!(offer["type"], "sub_offer");
    advance(&mut room, &mut clients, &mut now, 750, false);
    assert!(!room.peers.contains_key(&1));
    assert_eq!(room.peers.len(), 2);
}
