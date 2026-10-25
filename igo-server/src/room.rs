use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use axum::extract::ws::{close_code, CloseFrame, Message, WebSocket};
use igo_core::{Color, Game, Phase, Vertex};
use tokio::sync::{broadcast, watch};
use tokio::time::Instant;
use tracing::{debug, error, info, warn};

use crate::protocol::{ClientMsg, ConnId, PlayerToken, Seats, ServerMsg, Settings, Snapshot};
use crate::store::{Event, Saved, SeatRecord, Store, Table};

const HELLO_TIMEOUT: Duration = Duration::from_secs(30);

const BROADCAST_CAPACITY: usize = 64;

const MAX_NAME_LEN: usize = 40;

struct RoomState {
    game: Game,
    seats: [Option<SeatRecord>; 2],
    conns: HashMap<ConnId, PlayerToken>,
    names: HashMap<PlayerToken, String>,
    dead: HashSet<Vertex>,
    accepted: HashSet<Color>,
    /// Number of events written to the store so far.
    logged: usize,
}

impl RoomState {
    fn seat_of(&self, token: &str) -> Option<Color> {
        [Color::Black, Color::White].into_iter().find(|color| {
            self.seats[color.index()]
                .as_ref()
                .is_some_and(|holder| holder.token == token)
        })
    }

    fn seats_view(&self) -> Seats {
        let name = |c: Color| self.seats[c.index()].as_ref().map(|h| h.name.clone());
        Seats {
            black: name(Color::Black),
            white: name(Color::White),
        }
    }

    fn dead_sorted(&self) -> Vec<Vertex> {
        let mut v: Vec<_> = self.dead.iter().copied().collect();
        v.sort_unstable();
        v
    }

    fn accepted_sorted(&self) -> Vec<Color> {
        [Color::Black, Color::White]
            .into_iter()
            .filter(|c| self.accepted.contains(c))
            .collect()
    }

    fn dead_message(&self) -> ServerMsg {
        ServerMsg::Dead {
            vertices: self.dead_sorted(),
            accepted: self.accepted_sorted(),
            score: self.game.provisional_score(&self.dead),
            territory: igo_core::score::ownership(self.game.board(), &self.dead),
        }
    }

    fn table(&self) -> Table {
        Table {
            seats: self.seats.clone(),
            dead: self.dead_sorted(),
            accepted: self.accepted_sorted(),
        }
    }

    fn apply(&mut self, event: &Event) -> Result<()> {
        match event {
            Event::Move { color, mv } => {
                self.game.play(*color, *mv)?;
            }
            Event::Resume => {
                self.game.resume_play()?;
                self.dead.clear();
                self.accepted.clear();
            }
            Event::Finish { dead } => {
                self.game
                    .finish_with_dead(&dead.iter().copied().collect())?;
            }
        }
        Ok(())
    }
}

/// One game and everyone looking at it.
///
/// Every change to the state is published while the write lock is still
/// held, so all connections see events in exactly the order they happened,
/// and a snapshot taken under the lock is an exact cut of that stream.
pub struct Room {
    id: String,
    settings: Settings,
    state: RwLock<RoomState>,
    events: broadcast::Sender<ServerMsg>,
    next_conn: AtomicU64,
    leases: AtomicUsize,
    last_active: Mutex<Instant>,
    store: Option<Arc<Store>>,
}

/// Keeps a room from being swept while a request or socket is using it.
pub struct Lease(Arc<Room>);

impl Lease {
    /// Must be called while the room cannot be removed from the room map.
    pub(crate) fn new(room: Arc<Room>) -> Lease {
        room.leases.fetch_add(1, Ordering::SeqCst);
        room.touch();
        Lease(room)
    }
}

impl std::ops::Deref for Lease {
    type Target = Room;
    fn deref(&self) -> &Room {
        &self.0
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.0.touch();
        if self.0.leases.fetch_sub(1, Ordering::SeqCst) == 1 {
            if let Some(store) = &self.0.store {
                if let Err(e) = store.touch(&self.0.id) {
                    error!(room = %self.0.id, error = %e, "could not record activity");
                }
            }
        }
    }
}

impl Room {
    /// A brand-new room. `settings` must already be valid.
    pub fn new(id: String, settings: Settings, store: Option<Arc<Store>>) -> Room {
        let game = Game::with_handicap(settings.size, settings.komi, settings.handicap)
            .expect("settings were validated");
        let (events, _) = broadcast::channel(BROADCAST_CAPACITY);
        Room {
            id,
            settings,
            state: RwLock::new(RoomState {
                game,
                seats: [None, None],
                conns: HashMap::new(),
                names: HashMap::new(),
                dead: HashSet::new(),
                accepted: HashSet::new(),
                logged: 0,
            }),
            events,
            next_conn: AtomicU64::new(0),
            leases: AtomicUsize::new(0),
            last_active: Mutex::new(Instant::now()),
            store,
        }
    }

    /// Rebuilds a room by replaying its stored log.
    pub fn restore(id: String, saved: Saved, store: Option<Arc<Store>>) -> Result<Room> {
        saved.settings.validate().map_err(|e| anyhow!(e))?;
        let room = Room::new(id, saved.settings, store);
        {
            let mut state = room.write();
            for (seq, event) in saved.events.iter().enumerate() {
                state
                    .apply(event)
                    .with_context(|| format!("replaying event {seq}: {event:?}"))?;
                state.logged += 1;
            }
            for holder in saved.table.seats.iter().flatten() {
                state
                    .names
                    .insert(holder.token.clone(), holder.name.clone());
            }
            state.seats = saved.table.seats;
            state.dead = saved.table.dead.into_iter().collect();
            state.accepted = saved.table.accepted.into_iter().collect();
        }
        Ok(room)
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn settings(&self) -> Settings {
        self.settings
    }

    fn read(&self) -> RwLockReadGuard<'_, RoomState> {
        self.state.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write(&self) -> RwLockWriteGuard<'_, RoomState> {
        self.state.write().unwrap_or_else(|e| e.into_inner())
    }

    fn touch(&self) {
        *self.last_active.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    }

    /// Nobody is using the room and nothing has happened in it for `max_idle`.
    pub fn is_idle(&self, max_idle: Duration) -> bool {
        self.leases.load(Ordering::SeqCst) == 0
            && self
                .last_active
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .elapsed()
                > max_idle
    }

    pub fn sgf(&self) -> String {
        igo_core::sgf::to_sgf(&self.read().game)
    }

    /// Runs `change` under the write lock, then publishes the messages it
    /// returns before releasing the lock.
    fn mutate(
        &self,
        change: impl FnOnce(&mut RoomState) -> Result<Vec<ServerMsg>, String>,
    ) -> Result<(), String> {
        let mut state = self.write();
        let msgs = change(&mut state)?;
        for msg in msgs {
            let _ = self.events.send(msg);
        }
        Ok(())
    }

    /// Writes `event` to the log. Call with the write lock held.
    fn log(&self, state: &mut RoomState, event: Event) {
        if let Some(store) = &self.store {
            if let Err(e) = store.append(&self.id, state.logged, &event) {
                error!(room = %self.id, error = %e, "could not persist event");
            }
        }
        state.logged += 1;
    }

    /// Saves seats and scoring marks. Call with the write lock held.
    fn save(&self, state: &RoomState) {
        if let Some(store) = &self.store {
            if let Err(e) = store.save_table(&self.id, &state.table()) {
                error!(room = %self.id, error = %e, "could not persist seats");
            }
        }
    }

    /// Subscribes to events and takes a snapshot at exactly the same point.
    fn subscribe(&self, token: &str) -> (broadcast::Receiver<ServerMsg>, Snapshot) {
        let state = self.read();
        let rx = self.events.subscribe();
        let snapshot = Snapshot {
            size: self.settings.size,
            komi: self.settings.komi,
            handicap: self.settings.handicap,
            board: state.game.board().as_bytes(),
            to_play: state.game.to_play(),
            move_number: state.game.move_number(),
            captures: state.game.captures(),
            phase: state.game.phase().clone(),
            seats: state.seats_view(),
            dead: state.dead_sorted(),
            accepted: state.accepted_sorted(),
            you: state.seat_of(token),
            connections: state.conns.len(),
        };
        (rx, snapshot)
    }

    pub async fn on_connection(&self, socket: WebSocket, shutdown: watch::Receiver<bool>) {
        let id = self.next_conn.fetch_add(1, Ordering::Relaxed);
        debug!(room = %self.id, conn = id, "connected");

        if let Err(e) = self.handle(id, socket, shutdown).await {
            debug!(room = %self.id, conn = id, error = %e, "connection ended");
        }

        // Seats deliberately survive the disconnect; only presence changes.
        let _ = self.mutate(|state| {
            Ok(match state.conns.remove(&id) {
                Some(_) => vec![ServerMsg::Presence {
                    connections: state.conns.len(),
                }],
                None => vec![],
            })
        });
        debug!(room = %self.id, conn = id, "disconnected");
    }

    async fn handle(
        &self,
        id: ConnId,
        mut socket: WebSocket,
        mut shutdown: watch::Receiver<bool>,
    ) -> Result<()> {
        let token = self.await_hello(id, &mut socket).await?;

        let (mut events, snapshot) = self.subscribe(&token);
        let mut you = snapshot.you;
        socket
            .send(ServerMsg::Identity { conn: id, you }.into())
            .await?;
        socket
            .send(ServerMsg::Snapshot(Box::new(snapshot)).into())
            .await?;

        loop {
            tokio::select! {
                event = events.recv() => match event {
                    Ok(msg) => {
                        if matches!(msg, ServerMsg::Seats { .. }) {
                            let now = self.read().seat_of(&token);
                            if now != you {
                                you = now;
                                socket
                                    .send(ServerMsg::Identity { conn: id, you }.into())
                                    .await?;
                            }
                        }
                        socket.send(msg.into()).await?;
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!(room = %self.id, conn = id, skipped, "fell behind; resyncing");
                        let (rx, snapshot) = self.subscribe(&token);
                        events = rx;
                        you = snapshot.you;
                        socket
                            .send(ServerMsg::Snapshot(Box::new(snapshot)).into())
                            .await?;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                },
                incoming = socket.recv() => match incoming {
                    None => break,
                    Some(message) => {
                        self.touch();
                        self.handle_message(id, message?, &mut socket).await?;
                    }
                },
                // Also fires if the server dropped the sender, which means it is gone too.
                _ = async { drop(shutdown.wait_for(|&down| down).await) } => {
                    let frame = CloseFrame {
                        code: close_code::RESTART,
                        reason: "server restarting".into(),
                    };
                    socket.send(Message::Close(Some(frame))).await?;
                    break;
                }
            }
        }

        Ok(())
    }

    async fn await_hello(&self, id: ConnId, socket: &mut WebSocket) -> Result<PlayerToken> {
        let deadline = tokio::time::timeout(HELLO_TIMEOUT, async {
            loop {
                match socket.recv().await {
                    None => bail!("closed before identifying itself"),
                    Some(message) => {
                        if let Message::Text(text) = message? {
                            return Ok(text);
                        }
                    }
                }
            }
        });

        let text = deadline
            .await
            .map_err(|_| anyhow!("timed out waiting for hello"))??;

        let msg: ClientMsg = serde_json::from_str(&text).context("malformed hello")?;
        let ClientMsg::Hello { token, name } = msg else {
            bail!("first message must be hello");
        };
        if token.is_empty() || token.len() > 64 {
            bail!("invalid player token");
        }

        let _ = self.mutate(|state| {
            state.conns.insert(id, token.clone());
            let mut msgs = vec![ServerMsg::Presence {
                connections: state.conns.len(),
            }];
            msgs.extend(self.rename(state, &token, name));
            Ok(msgs)
        });
        Ok(token)
    }

    fn rename(&self, state: &mut RoomState, token: &str, name: String) -> Option<ServerMsg> {
        let name: String = name.chars().take(MAX_NAME_LEN).collect();
        let name = if name.trim().is_empty() {
            "Anonymous".to_string()
        } else {
            name
        };

        state.names.insert(token.to_owned(), name.clone());
        let color = state.seat_of(token)?;
        let holder = state.seats[color.index()].as_mut()?;
        if holder.name == name {
            return None;
        }
        holder.name = name;
        self.save(state);
        Some(ServerMsg::Seats {
            seats: state.seats_view(),
        })
    }

    async fn handle_message(
        &self,
        id: ConnId,
        message: Message,
        socket: &mut WebSocket,
    ) -> Result<()> {
        let Message::Text(text) = message else {
            return Ok(());
        };
        let msg: ClientMsg = serde_json::from_str(&text).context("malformed message")?;

        if let Err(reason) = self.apply(id, msg) {
            debug!(room = %self.id, conn = id, %reason, "refused");
            socket.send(ServerMsg::Rejected { reason }.into()).await?;
        }
        Ok(())
    }

    fn apply(&self, id: ConnId, msg: ClientMsg) -> Result<(), String> {
        self.mutate(|state| {
            let token = state
                .conns
                .get(&id)
                .cloned()
                .ok_or("connection has not identified itself")?;

            match msg {
                ClientMsg::Hello {
                    token: sent,
                    name: new_name,
                } => {
                    if sent != token {
                        return Err("cannot change identity on an open connection".into());
                    }
                    Ok(self.rename(state, &token, new_name).into_iter().collect())
                }

                ClientMsg::ClaimSeat { color } => {
                    if let Some(held) = state.seat_of(&token) {
                        return Err(if held == color {
                            "you already have that seat".into()
                        } else {
                            "you are already playing the other colour".into()
                        });
                    }
                    if state.seats[color.index()].is_some() {
                        return Err("that seat is taken".into());
                    }
                    let name = state
                        .names
                        .get(&token)
                        .cloned()
                        .unwrap_or_else(|| "Anonymous".into());
                    state.seats[color.index()] = Some(SeatRecord { token, name });
                    self.save(state);

                    info!(room = %self.id, ?color, "seat claimed");
                    Ok(vec![ServerMsg::Seats {
                        seats: state.seats_view(),
                    }])
                }

                ClientMsg::ReleaseSeat => {
                    let color = state.seat_of(&token).ok_or("you do not have a seat")?;
                    if state.game.move_number() > 0 {
                        return Err("the game has already started".into());
                    }
                    state.seats[color.index()] = None;
                    self.save(state);
                    Ok(vec![ServerMsg::Seats {
                        seats: state.seats_view(),
                    }])
                }

                ClientMsg::Play { move_number, mv } => {
                    let color = state
                        .seat_of(&token)
                        .ok_or("you are watching this game, not playing it")?;

                    let current = state.game.move_number();
                    if move_number != current {
                        return Err(format!(
                            "stale move: sent at move {move_number}, game is at {current}"
                        ));
                    }

                    let applied = state.game.play(color, mv).map_err(|e| e.to_string())?;
                    self.log(state, Event::Move { color, mv });

                    let mut msgs = vec![ServerMsg::Moved {
                        applied,
                        to_play: state.game.to_play(),
                        captures: state.game.captures(),
                        phase: state.game.phase().clone(),
                    }];
                    if matches!(state.game.phase(), Phase::Scoring) {
                        msgs.push(state.dead_message());
                    }
                    Ok(msgs)
                }

                ClientMsg::ToggleDead { vertex } => {
                    if !matches!(state.game.phase(), Phase::Scoring) {
                        return Err("the game is not being scored".into());
                    }
                    state
                        .seat_of(&token)
                        .ok_or("only the players may mark stones")?;

                    let group = state
                        .game
                        .board()
                        .group_at(vertex)
                        .ok_or("there is no stone there")?;

                    let all_dead = group.stones.iter().all(|v| state.dead.contains(v));
                    for v in &group.stones {
                        if all_dead {
                            state.dead.remove(v);
                        } else {
                            state.dead.insert(*v);
                        }
                    }
                    state.accepted.clear();
                    self.save(state);
                    Ok(vec![state.dead_message()])
                }

                ClientMsg::AcceptScore => {
                    if !matches!(state.game.phase(), Phase::Scoring) {
                        return Err("the game is not being scored".into());
                    }
                    let color = state
                        .seat_of(&token)
                        .ok_or("only the players may agree the score")?;
                    if state.seats.iter().any(Option::is_none) {
                        return Err("both seats must be filled to finish the game".into());
                    }

                    state.accepted.insert(color);
                    if state.accepted.len() < 2 {
                        self.save(state);
                        return Ok(vec![state.dead_message()]);
                    }

                    let dead = state.dead_sorted();
                    state
                        .game
                        .finish_with_dead(&state.dead)
                        .map_err(|e| e.to_string())?;
                    self.log(state, Event::Finish { dead });
                    self.save(state);
                    Ok(vec![
                        state.dead_message(),
                        ServerMsg::PhaseChanged {
                            phase: state.game.phase().clone(),
                        },
                    ])
                }

                // Only arriving matters: it keeps an idle host from putting the server to sleep.
                ClientMsg::Heartbeat => Ok(vec![]),
                ClientMsg::ResumePlay => {
                    state
                        .seat_of(&token)
                        .ok_or("only the players may resume the game")?;
                    state
                        .apply(&Event::Resume)
                        .map_err(|_| "the game is not being scored".to_string())?;
                    self.log(state, Event::Resume);
                    self.save(state);
                    Ok(vec![
                        ServerMsg::PhaseChanged {
                            phase: state.game.phase().clone(),
                        },
                        state.dead_message(),
                    ])
                }
            }
        })
    }
}
