use antfarm_core::{ClientMessage, GameState, JoinAck, ServerMessage, TICK_MILLIS};
use anyhow::Result;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::{Mutex, mpsc},
    time,
};

type ClientTx = mpsc::UnboundedSender<ServerMessage>;

#[derive(Clone)]
struct ServerState {
    game: Arc<Mutex<GameState>>,
    clients: Arc<Mutex<HashMap<u8, ClientTx>>>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:7000").await?;
    let state = ServerState {
        game: Arc::new(Mutex::new(GameState::new())),
        clients: Arc::new(Mutex::new(HashMap::new())),
    };

    {
        let tick_state = state.clone();
        tokio::spawn(async move {
            let mut ticker = time::interval(Duration::from_millis(TICK_MILLIS));
            loop {
                ticker.tick().await;
                {
                    let mut game = tick_state.game.lock().await;
                    game.tick();
                }
                if let Err(error) = broadcast_snapshot(&tick_state).await {
                    eprintln!("broadcast error: {error}");
                }
            }
        });
    }

    println!("antfarm-server listening on 127.0.0.1:7000");

    loop {
        let (stream, _) = listener.accept().await?;
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(error) = handle_client(stream, state).await {
                eprintln!("client disconnected with error: {error}");
            }
        });
    }
}

async fn handle_client(stream: TcpStream, state: ServerState) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    let (tx, mut rx) = mpsc::unbounded_channel::<ServerMessage>();

    let mut player_id = None;

    let writer_task = tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            let payload = serde_json::to_string(&message)?;
            writer.write_all(payload.as_bytes()).await?;
            writer.write_all(b"\n").await?;
        }
        Ok::<(), anyhow::Error>(())
    });

    while let Some(line) = lines.next_line().await? {
        let message: ClientMessage = serde_json::from_str(&line)?;
        match message {
            ClientMessage::Join { name } => {
                if player_id.is_some() {
                    continue;
                }

                let joined = {
                    let mut game = state.game.lock().await;
                    game.add_player(name)
                };

                match joined {
                    Ok((id, snapshot)) => {
                        state.clients.lock().await.insert(id, tx.clone());
                        player_id = Some(id);
                        tx.send(ServerMessage::Joined(JoinAck {
                            player_id: id,
                            snapshot,
                        }))?;
                        broadcast_snapshot(&state).await?;
                    }
                    Err(message) => {
                        tx.send(ServerMessage::Error { message })?;
                    }
                }
            }
            ClientMessage::Action(action) => {
                let Some(id) = player_id else {
                    tx.send(ServerMessage::Error {
                        message: "Join before sending actions".to_string(),
                    })?;
                    continue;
                };

                {
                    let mut game = state.game.lock().await;
                    game.apply_action(id, action);
                }
                broadcast_snapshot(&state).await?;
            }
        }
    }

    if let Some(id) = player_id {
        state.clients.lock().await.remove(&id);
        {
            let mut game = state.game.lock().await;
            game.remove_player(id);
        }
        broadcast_snapshot(&state).await?;
    }

    writer_task.abort();
    Ok(())
}

async fn broadcast_snapshot(state: &ServerState) -> Result<()> {
    let snapshot = {
        let game = state.game.lock().await;
        game.snapshot()
    };

    let clients = state.clients.lock().await;
    for tx in clients.values() {
        let _ = tx.send(ServerMessage::Snapshot(snapshot.clone()));
    }
    Ok(())
}
