use crate::protocol::{self, ClientMessage, MAX_MESSAGE};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    net::TcpStream,
    sync::{Semaphore, mpsc},
    time::timeout,
};
use tokio_tungstenite::{
    accept_hdr_async_with_config,
    tungstenite::{
        Message,
        handshake::server::{Request, Response},
        protocol::WebSocketConfig,
    },
};

pub enum Command {
    Join {
        id: u64,
        name: String,
        tx: mpsc::Sender<Value>,
    },
    Message {
        id: u64,
        message: ClientMessage,
    },
    Left(u64),
}

pub struct RateLimit {
    start: Instant,
    count: u32,
}
impl RateLimit {
    pub fn new(now: Instant) -> Self {
        Self {
            start: now,
            count: 0,
        }
    }
    pub fn allow(&mut self, now: Instant) -> bool {
        if now.duration_since(self.start) >= Duration::from_secs(1) {
            self.start = now;
            self.count = 0;
        }
        self.count += 1;
        self.count <= 60
    }
}

// Tungstenite requires this concrete HTTP error response in its callback API.
#[allow(clippy::result_large_err)]
fn check_path(
    request: &Request,
    response: Response,
) -> std::result::Result<Response, tokio_tungstenite::tungstenite::handshake::server::ErrorResponse>
{
    if request.uri().path() == "/ws" {
        Ok(response)
    } else {
        let mut response =
            tokio_tungstenite::tungstenite::http::Response::new(Some("Not found".into()));
        *response.status_mut() = tokio_tungstenite::tungstenite::http::StatusCode::NOT_FOUND;
        Err(response)
    }
}

pub async fn serve(
    stream: TcpStream,
    id: u64,
    commands: mpsc::Sender<Command>,
    slots: Arc<Semaphore>,
) {
    let Ok(_permit) = slots.try_acquire_owned() else {
        return;
    };
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE))
        .max_frame_size(Some(MAX_MESSAGE));
    let handshake = accept_hdr_async_with_config(stream, check_path, Some(config));
    let Ok(Ok(mut ws)) = timeout(Duration::from_secs(5), handshake).await else {
        return;
    };
    let first = timeout(Duration::from_secs(10), ws.next()).await;
    let parsed = match first {
        Ok(Some(Ok(Message::Text(text)))) => protocol::parse(&text),
        _ => Err("Join must be the first message"),
    };
    let name = match parsed {
        Ok(ClientMessage::Join { name, protocol })
            if protocol::validate_join(&name, protocol).is_ok() =>
        {
            name
        }
        _ => {
            let _ = timeout(
                Duration::from_secs(2),
                ws.send(Message::Text(
                    json!({"type":"error", "message":"Invalid join or protocol", "fatal":true})
                        .to_string()
                        .into(),
                )),
            )
            .await;
            let _ = timeout(Duration::from_secs(2), ws.close(None)).await;
            return;
        }
    };
    let (tx, mut rx) = mpsc::channel::<Value>(32);
    if commands.send(Command::Join { id, name, tx }).await.is_err() {
        return;
    }
    let mut rate = RateLimit::new(Instant::now());
    let mut probe = tokio::time::interval(Duration::from_secs(4));
    probe.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    probe.tick().await;
    let mut last_pong = tokio::time::Instant::now();
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(last_pong + Duration::from_secs(15)) => break,
            _ = probe.tick() => {
                if !matches!(timeout(Duration::from_secs(2), ws.send(Message::Ping(Default::default()))).await, Ok(Ok(()))) { break; }
            }
            outgoing = rx.recv() => {
                let Some(value) = outgoing else { break; };
                let fatal = value.get("fatal").and_then(Value::as_bool) == Some(true);
                if !matches!(timeout(Duration::from_secs(2), ws.send(Message::Text(value.to_string().into()))).await, Ok(Ok(()))) || fatal { break; }
            }
            incoming = ws.next() => {
                if !rate.allow(Instant::now()) { break; }
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        match protocol::parse(&text) {
                            Ok(ClientMessage::Leave {}) => break,
                            Ok(message) => if commands.send(Command::Message { id, message }).await.is_err() { break; },
                            Err(_) => break,
                        }
                    }
                    Some(Ok(Message::Ping(_))) => { if !matches!(timeout(Duration::from_secs(2), ws.flush()).await, Ok(Ok(()))) { break; } }
                    Some(Ok(Message::Pong(_))) => last_pong = tokio::time::Instant::now(),
                    _ => break,
                }
            }
        }
    }
    let _ = commands.send(Command::Left(id)).await;
    let _ = timeout(Duration::from_secs(2), ws.close(None)).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn limits_bursts_and_recovers() {
        let now = Instant::now();
        let mut rate = RateLimit::new(now);
        for _ in 0..60 {
            assert!(rate.allow(now));
        }
        assert!(!rate.allow(now));
        assert!(rate.allow(now + Duration::from_secs(1)));
    }
}

#[cfg(test)]
mod socket_tests {
    use super::*;
    use tokio::net::TcpListener;
    use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
    type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

    async fn send(ws: &mut Socket, value: Value) {
        ws.send(Message::Text(value.to_string().into()))
            .await
            .unwrap();
    }
    async fn receive(ws: &mut Socket) -> Value {
        let message = timeout(Duration::from_secs(3), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        serde_json::from_str(message.to_text().unwrap()).unwrap()
    }
    async fn closed(ws: &mut Socket) {
        let message = timeout(Duration::from_secs(3), ws.next()).await.unwrap();
        assert!(matches!(
            message,
            None | Some(Ok(Message::Close(_))) | Some(Err(_))
        ));
    }

    #[tokio::test]
    async fn silent_websocket_is_removed_even_if_the_room_keeps_it_open() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/ws", listener.local_addr().unwrap());
        let (commands, mut received) = mpsc::channel(4);
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            serve(tcp, 7, commands, Arc::new(Semaphore::new(1))).await;
        });
        let (mut ws, _) = connect_async(url).await.unwrap();
        send(
            &mut ws,
            json!({"type":"join", "name":"Silent", "protocol":protocol::PROTOCOL}),
        )
        .await;
        let Some(Command::Join { tx, .. }) = received.recv().await else {
            panic!("Expected a join");
        };
        // Keep both ends open without polling the client, so it cannot return a Pong.
        let next = timeout(Duration::from_secs(18), received.recv())
            .await
            .unwrap();
        assert!(matches!(next, Some(Command::Left(7))));
        server.await.unwrap();
        drop((tx, ws));
    }

    #[tokio::test]
    async fn websocket_join_state_ping_invalid_input_and_disconnect() {
        str0m::crypto::from_feature_flags().install_process_default();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/ws", listener.local_addr().unwrap());
        let (tx, mut rx) = mpsc::channel(128);
        let slots = Arc::new(Semaphore::new(32));
        let server = tokio::spawn(async move {
            let mut room = crate::room::Room::new("127.0.0.1:7882".parse().unwrap());
            let mut id = 0;
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        id += 1;
                        tokio::spawn(serve(accepted.unwrap().0, id, tx.clone(), slots.clone()));
                    }
                    Some(command) = rx.recv() => { room.command(command); room.pump(); }
                }
            }
        });
        let (mut first, _) = connect_async(&url).await.unwrap();
        send(
            &mut first,
            json!({"type":"join","name":"First","protocol":protocol::PROTOCOL}),
        )
        .await;
        assert_eq!(receive(&mut first).await["type"], "joined");
        let (mut second, _) = connect_async(&url).await.unwrap();
        send(
            &mut second,
            json!({"type":"join","name":"Second","protocol":protocol::PROTOCOL}),
        )
        .await;
        assert_eq!(
            receive(&mut second).await["peers"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(receive(&mut first).await["type"], "peer_joined");
        send(
            &mut second,
            json!({"type":"state","mic":true,"cam":false,"deaf":false}),
        )
        .await;
        assert_eq!(receive(&mut first).await["mic"], true);
        send(&mut first, json!({"type":"ping"})).await;
        assert_eq!(receive(&mut first).await["type"], "pong");
        send(&mut second, json!({"type":"state","mic":"bad"})).await;
        closed(&mut second).await;
        assert_eq!(receive(&mut first).await["type"], "peer_left");
        let (mut invalid, _) = connect_async(&url).await.unwrap();
        send(
            &mut invalid,
            json!({"type":"join","name":"Invalid","protocol":1}),
        )
        .await;
        assert_eq!(receive(&mut invalid).await["fatal"], true);
        closed(&mut invalid).await;
        let (mut oversized, _) = connect_async(&url).await.unwrap();
        send(
            &mut oversized,
            json!({"type":"join","name":"Large","protocol":protocol::PROTOCOL}),
        )
        .await;
        receive(&mut oversized).await;
        receive(&mut first).await;
        oversized
            .send(Message::Text("x".repeat(MAX_MESSAGE + 1).into()))
            .await
            .unwrap();
        closed(&mut oversized).await;
        assert_eq!(receive(&mut first).await["type"], "peer_left");
        send(&mut first, json!({"type":"leave"})).await;
        closed(&mut first).await;
        server.abort();
    }
}
