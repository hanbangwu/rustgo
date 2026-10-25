use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Path, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use dashmap::DashMap;
use rand::RngExt;
use serde::Serialize;
use tokio::sync::watch;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use tracing::{error, info, warn};

pub mod protocol;
pub mod room;
pub mod store;

use protocol::{NewRoom, Settings};
use room::{Lease, Room};
use store::Store;

const MAX_ROOM_ID_LEN: usize = 32;
const ROOM_ID_LEN: usize = 8;
const ROOM_ID_CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// A room nobody has used for this long is deleted.
    pub max_idle: Duration,
    pub sweep_interval: Duration,
    /// Refuse to create rooms beyond this many in memory.
    pub max_rooms: usize,
    pub static_dir: PathBuf,
    /// SQLite file to keep games in across restarts; memory only if unset.
    pub database: Option<PathBuf>,
    /// Origins allowed to call the API from a frontend hosted elsewhere.
    pub allowed_origins: Vec<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            max_idle: Duration::from_secs(24 * 3600),
            sweep_interval: Duration::from_secs(3600),
            max_rooms: 10_000,
            static_dir: PathBuf::from("web/build"),
            database: None,
            allowed_origins: Vec::new(),
        }
    }
}

/// Closes open game connections when the server is going down, so clients
/// can reconnect to its replacement straight away.
#[derive(Clone)]
pub struct Shutdown(Arc<watch::Sender<bool>>);

impl Shutdown {
    pub fn new() -> Self {
        Shutdown(Arc::new(watch::Sender::new(false)))
    }

    /// Tells every open connection, and any that opens later, to close.
    pub fn trigger(&self) {
        self.0.send_replace(true);
    }

    /// Waits up to `grace` for the connections to finish closing.
    pub async fn drained(&self, grace: Duration) {
        let _ = tokio::time::timeout(grace, self.0.closed()).await;
    }

    fn subscribe(&self) -> watch::Receiver<bool> {
        self.0.subscribe()
    }
}

impl Default for Shutdown {
    fn default() -> Self {
        Shutdown::new()
    }
}

#[derive(Clone)]
struct AppState {
    rooms: Arc<DashMap<String, Arc<Room>>>,
    store: Option<Arc<Store>>,
    config: Arc<ServerConfig>,
    shutdown: Shutdown,
    start_time: u64,
}

impl AppState {
    /// Finds a room in memory or, failing that, in the store.
    fn lease(&self, id: &str) -> Option<Lease> {
        // Leasing under the map's shard lock means the sweeper can never
        // remove a room between finding it and leasing it.
        if let Some(lease) = self.rooms.get(id).map(|r| Lease::new(Arc::clone(&r))) {
            return Some(lease);
        }

        let store = self.store.as_ref()?;
        let saved = match store.load(id) {
            Ok(saved) => saved?,
            Err(e) => {
                error!(room = id, error = %e, "could not load room");
                return None;
            }
        };
        let room = match Room::restore(id.to_owned(), saved, Some(Arc::clone(store))) {
            Ok(room) => room,
            Err(e) => {
                error!(room = id, error = %e, "stored room is corrupt");
                return None;
            }
        };
        info!(room = id, "restored room");
        let entry = self.rooms.entry(id.to_owned()).or_insert(Arc::new(room));
        Some(Lease::new(Arc::clone(&entry)))
    }

    fn create(&self, settings: Settings) -> Result<String, (StatusCode, &'static str)> {
        settings
            .validate()
            .map_err(|e| (StatusCode::UNPROCESSABLE_ENTITY, e))?;
        if self.rooms.len() >= self.config.max_rooms {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "too many games in progress",
            ));
        }

        loop {
            let id = random_room_id();
            if let Some(store) = &self.store {
                match store.exists(&id) {
                    Ok(false) => {}
                    Ok(true) => continue,
                    Err(e) => {
                        error!(error = %e, "could not check room id");
                        return Err((StatusCode::INTERNAL_SERVER_ERROR, "database error"));
                    }
                }
            }
            let dashmap::Entry::Vacant(slot) = self.rooms.entry(id.clone()) else {
                continue;
            };
            if let Some(store) = &self.store {
                if let Err(e) = store.create(&id, &settings) {
                    error!(error = %e, "could not create room");
                    return Err((StatusCode::INTERNAL_SERVER_ERROR, "database error"));
                }
            }
            slot.insert(Arc::new(Room::new(
                id.clone(),
                settings,
                self.store.clone(),
            )));
            info!(room = %id, ?settings, "created room");
            return Ok(id);
        }
    }
}

fn random_room_id() -> String {
    let mut rng = rand::rng();
    (0..ROOM_ID_LEN)
        .map(|_| ROOM_ID_CHARS[rng.random_range(0..ROOM_ID_CHARS.len())] as char)
        .collect()
}

fn valid_room_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ROOM_ID_LEN
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub fn app(config: ServerConfig) -> anyhow::Result<Router> {
    app_with_shutdown(config, Shutdown::new())
}

pub fn app_with_shutdown(config: ServerConfig, shutdown: Shutdown) -> anyhow::Result<Router> {
    let cors = cors_layer(&config.allowed_origins)?;
    let store = config
        .database
        .as_deref()
        .map(Store::open)
        .transpose()?
        .map(Arc::new);
    let static_dir = config.static_dir.clone();

    let state = AppState {
        rooms: Arc::new(DashMap::new()),
        store,
        config: Arc::new(config),
        shutdown,
        start_time: SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    };

    tokio::spawn(cleaner(state.clone()));

    let index = static_dir.join("index.html");
    let files = ServeDir::new(static_dir).fallback(ServeFile::new(index));

    let router = Router::new()
        .route("/api/rooms", post(create_handler))
        .route("/api/rooms/{id}", get(room_handler))
        .route("/api/socket/{id}", get(socket_handler))
        .route("/api/sgf/{id}", get(sgf_handler))
        .route("/api/stats", get(stats_handler))
        .route("/api/health", get(|| async { "ok" }))
        .with_state(state)
        .fallback_service(files);
    Ok(match cors {
        Some(cors) => router.layer(cors),
        None => router,
    })
}

/// None when the frontend is served from the same origin as the API.
fn cors_layer(origins: &[String]) -> anyhow::Result<Option<CorsLayer>> {
    if origins.is_empty() {
        return Ok(None);
    }
    let origins = origins
        .iter()
        .map(|origin| {
            HeaderValue::from_str(origin.trim_end_matches('/'))
                .map_err(|_| anyhow::anyhow!("invalid allowed origin {origin:?}"))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(Some(
        CorsLayer::new()
            .allow_origin(origins)
            .allow_methods([Method::GET, Method::POST])
            .allow_headers([header::CONTENT_TYPE]),
    ))
}

#[derive(Serialize)]
struct Created {
    id: String,
    #[serde(flatten)]
    settings: Settings,
}

async fn create_handler(State(state): State<AppState>, Json(req): Json<NewRoom>) -> Response {
    let settings = Settings::from(req);
    match state.create(settings) {
        Ok(id) => (StatusCode::CREATED, Json(Created { id, settings })).into_response(),
        Err(rejection) => rejection.into_response(),
    }
}

fn find(state: &AppState, id: &str) -> Result<Lease, (StatusCode, &'static str)> {
    if !valid_room_id(id) {
        return Err((StatusCode::BAD_REQUEST, "invalid room id"));
    }
    state
        .lease(id)
        .ok_or((StatusCode::NOT_FOUND, "no such game"))
}

async fn room_handler(Path(id): Path<String>, State(state): State<AppState>) -> Response {
    match find(&state, &id) {
        Ok(room) => Json(Created {
            id,
            settings: room.settings(),
        })
        .into_response(),
        Err(rejection) => rejection.into_response(),
    }
}

async fn socket_handler(
    Path(id): Path<String>,
    State(state): State<AppState>,
    ws: WebSocketUpgrade,
) -> Response {
    match find(&state, &id) {
        Ok(room) => {
            let shutdown = state.shutdown.subscribe();
            ws.on_upgrade(move |socket| async move { room.on_connection(socket, shutdown).await })
        }
        Err(rejection) => rejection.into_response(),
    }
}

async fn sgf_handler(Path(id): Path<String>, State(state): State<AppState>) -> Response {
    let room = match find(&state, &id) {
        Ok(room) => room,
        Err(rejection) => return rejection.into_response(),
    };
    (
        [
            (header::CONTENT_TYPE, "application/x-go-sgf".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{id}.sgf\""),
            ),
        ],
        room.sgf(),
    )
        .into_response()
}

#[derive(Serialize)]
struct Stats {
    start_time: u64,
    rooms: usize,
}

async fn stats_handler(State(state): State<AppState>) -> Json<Stats> {
    Json(Stats {
        start_time: state.start_time,
        rooms: state.rooms.len(),
    })
}

/// Deletes rooms nobody has used for `max_idle`, from memory and the store.
async fn cleaner(state: AppState) {
    let max_idle = state.config.max_idle;
    loop {
        tokio::time::sleep(state.config.sweep_interval).await;

        let before = state.rooms.len();
        state.rooms.retain(|id, room| {
            if !room.is_idle(max_idle) {
                return true;
            }
            // Deleting inside `retain` holds the shard lock, so the room
            // cannot be reloaded from the store halfway through.
            if let Some(store) = &state.store {
                if let Err(e) = store.delete(id) {
                    warn!(room = %id, error = %e, "could not delete expired room");
                }
            }
            false
        });
        let swept = before.saturating_sub(state.rooms.len());

        // Rooms that were never loaded since the last restart.
        let mut dormant = 0;
        if let Some(store) = &state.store {
            match store.stale(max_idle.as_secs()) {
                Ok(ids) => {
                    for id in ids.iter().filter(|id| !state.rooms.contains_key(*id)) {
                        if let Err(e) = store.delete(id) {
                            warn!(room = %id, error = %e, "could not delete expired room");
                        }
                        dormant += 1;
                    }
                }
                Err(e) => warn!(error = %e, "could not list expired rooms"),
            }
        }

        if swept + dormant > 0 {
            info!(swept, dormant, "reclaimed idle rooms");
        }
    }
}
