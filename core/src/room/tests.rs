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
    rx: mpsc::Receiver<Value>,
    received: Vec<(Mid, Vec<u8>)>,
    mapping: Vec<Value>,
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
        changes.add_media(MediaKind::Video, Direction::SendOnly, None, None, None);
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
            rx,
            received: Vec::new(),
            mapping: Vec::new(),
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
                "sub_offer" => {
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
}
fn exchange(room: &mut Room, clients: &mut [Client], now: Instant) {
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
        while let Some((source, destination, bytes)) = queue.pop_front() {
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
        exchange(room, clients, *now);
        if audio {
            for client in clients.iter_mut() {
                client.write_audio(*now, tick);
            }
            exchange(room, clients, *now);
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
        assert_eq!(client.mapping.len(), 2);
        assert!(
            !client.received.is_empty(),
            "Peer {} received no audio",
            client.id
        );
        for track in &client.mapping {
            assert!(client.received.iter().any(|(mid, data)| mid.to_string()
                == track["mid"].as_str().unwrap()
                && data == &[0xf8, 0xff, 0xfe]));
        }
    }
    clients.pop();
    room.command(Command::Left(3));
    exchange(&mut room, &mut clients, now);
    for client in &clients {
        assert_eq!(client.mapping.len(), 1);
    }
    clients.push(Client::new(&mut room, 4, now));
    exchange(&mut room, &mut clients, now);
    advance(&mut room, &mut clients, &mut now, 100, false);
    for client in &mut clients {
        client.received.clear();
    }
    advance(&mut room, &mut clients, &mut now, 30, true);
    for client in &clients {
        assert_eq!(client.mapping.len(), 2);
        assert!(!client.received.is_empty());
        assert!(client.mapping.iter().all(|t| t["peer"] != "3"));
    }
    assert!(room.peers.values().all(|p| p.slots.len() == 2));
}

#[test]
fn defers_subscription_changes_until_answer() {
    let mut room = test_room();
    let mut rx = join(&mut room, 1);
    rx.try_recv().unwrap();
    let peer = room.peers.get_mut(&1).unwrap();
    peer.subscriptions(&[2]);
    let offer = rx.try_recv().unwrap();
    peer.subscriptions(&[2, 3]);
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
    room.peers.get_mut(&1).unwrap().subscriptions(&[2, 3]);
    assert_eq!(
        rx.try_recv().unwrap()["tracks"].as_array().unwrap().len(),
        2
    );
}
