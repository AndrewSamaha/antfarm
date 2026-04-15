use antfarm_core::{ClientMessage, ServerMessage, Snapshot, World, default_server_config};
use anyhow::{Context, Result};
use crossterm::{event::Event, event::EventStream};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
    sync::mpsc,
    time::{Duration, timeout},
};

pub(crate) const RECONNECT_ATTEMPT_TIMEOUT: Duration = Duration::from_millis(900);

pub(crate) struct Connection {
    pub(crate) writer: tokio::net::tcp::OwnedWriteHalf,
    network_rx: mpsc::UnboundedReceiver<ServerMessage>,
}

pub(crate) async fn connect_session(player_name: &str, client_token: &str) -> Result<Connection> {
    let stream = timeout(
        RECONNECT_ATTEMPT_TIMEOUT,
        TcpStream::connect("127.0.0.1:7000"),
    )
    .await
    .context("timed out connecting to antfarm-server")?
    .context("connect to antfarm-server on 127.0.0.1:7000")?;

    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    let join = serde_json::to_string(&ClientMessage::Join {
        name: player_name.to_string(),
        token: client_token.to_string(),
    })?;
    writer.write_all(join.as_bytes()).await?;
    writer.write_all(b"\n").await?;

    let (network_tx, network_rx) = mpsc::unbounded_channel::<ServerMessage>();
    tokio::spawn(async move {
        while let Ok(Some(line)) = lines.next_line().await {
            match serde_json::from_str::<ServerMessage>(&line) {
                Ok(message) => {
                    let _ = network_tx.send(message);
                }
                Err(error) => {
                    let _ = network_tx.send(ServerMessage::Error {
                        message: error.to_string(),
                    });
                    break;
                }
            }
        }
    });

    Ok(Connection { writer, network_rx })
}

pub(crate) async fn recv_server_message(
    connection: &mut Option<Connection>,
) -> Option<ServerMessage> {
    let connection = connection.as_mut()?;
    connection.network_rx.recv().await
}

pub(crate) async fn send_action(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    action: antfarm_core::Action,
) -> Result<()> {
    let payload = serde_json::to_string(&ClientMessage::Action(action))?;
    writer.write_all(payload.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    Ok(())
}

pub(crate) async fn send_message(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    message: ClientMessage,
) -> Result<()> {
    let payload = serde_json::to_string(&message)?;
    writer.write_all(payload.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    Ok(())
}

pub(crate) async fn tokio_stream_event(events: &mut EventStream) -> Result<Option<Event>> {
    use futures_util::StreamExt;

    Ok(events.next().await.transpose()?)
}

pub(crate) fn offline_snapshot() -> Snapshot {
    Snapshot {
        tick: 0,
        world: World::empty(1, 1),
        players: Vec::new(),
        npcs: Vec::new(),
        placed_art: Vec::new(),
        event_log: Vec::new(),
        config: default_server_config(),
        simulation_paused: false,
    }
}
