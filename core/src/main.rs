#![forbid(unsafe_code)]
mod config;
mod protocol;
mod room;
mod signaling;

use std::{
    net::Ipv4Addr,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    net::{TcpListener, UdpSocket},
    sync::{Semaphore, mpsc},
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();
    let Some(config) = config::Config::parse(std::env::args().skip(1))? else {
        return Ok(());
    };
    str0m::crypto::from_feature_flags().install_process_default();
    let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, config.port)).await?;
    let udp = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, config.advertised.port())).await?;
    let (tx, mut rx) = mpsc::channel(128);
    let slots = Arc::new(Semaphore::new(32));
    let mut room = room::Room::new(config.advertised);
    let mut next_id = 0u64;
    let mut buffer = vec![0u8; 65536];
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        signal_port = config.port,
        udp_port = config.advertised.port(),
        "SFU started"
    );
    loop {
        for (destination, bytes) in room.pump() {
            if !matches!(
                tokio::time::timeout(Duration::from_millis(10), udp.send_to(&bytes, destination))
                    .await,
                Ok(Ok(_))
            ) {
                tracing::debug!("UDP output dropped");
            }
        }
        let deadline = tokio::time::Instant::from_std(room.deadline());
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                next_id = next_id.checked_add(1).ok_or("Connection identifier exhausted")?;
                tokio::spawn(signaling::serve(stream, next_id, tx.clone(), slots.clone()));
            }
            Some(command) = rx.recv() => room.command(command),
            received = udp.recv_from(&mut buffer) => {
                match received {
                    Ok((len, source)) => room.receive(source, &buffer[..len], Instant::now()),
                    Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {},
                    Err(error) => return Err(error.into()),
                }
            }
            _ = tokio::time::sleep_until(deadline) => room.timeout(Instant::now()),
            _ = tokio::signal::ctrl_c() => { tracing::info!("SFU stopped"); break; }
        }
    }
    Ok(())
}
