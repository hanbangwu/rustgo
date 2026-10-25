use std::cell::RefCell;

use crate::protocol::{ClientMsg, ConnId, PlayerToken, RoomInfo, ServerMsg, Settings};
use crate::record::{Event, Saved, Table};
use crate::state::{parse_hello, Outcome, RoomState};
use igo_core::Color;
use serde::{Deserialize, Serialize};
use worker::*;

const CLOSE_PROTOCOL: u16 = 1008;

const HEARTBEAT: &str = r#"{"type":"heartbeat"}"#;

#[derive(Debug, Default, Serialize, Deserialize)]
struct Attachment {
    conn: ConnId,
    token: Option<PlayerToken>,
    you: Option<Color>,
}

#[derive(Deserialize)]
struct RoomRow {
    settings: String,
    tbl: String,
}

#[derive(Deserialize)]
struct EventRow {
    event: String,
}

#[derive(Deserialize)]
struct Count {
    n: i64,
}

fn rust_error(e: impl std::fmt::Display) -> Error {
    Error::RustError(e.to_string())
}

fn attachment(ws: &WebSocket) -> Attachment {
    ws.deserialize_attachment()
        .ok()
        .flatten()
        .unwrap_or_default()
}

#[durable_object]
pub struct Room {
    state: State,
    room: RefCell<Option<RoomState>>,
}

impl DurableObject for Room {
    fn new(state: State, _env: Env) -> Self {
        if let Ok(pair) = worker_sys::WebSocketRequestResponsePair::new(HEARTBEAT, HEARTBEAT) {
            state.set_websocket_auto_response(&pair);
        }
        Room {
            state,
            room: RefCell::new(None),
        }
    }

    async fn fetch(&self, mut req: Request) -> Result<Response> {
        let path = req.path();
        let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
        let Some(&id) = segments.last() else {
            return Response::error("not found", 404);
        };

        match (req.method(), segments.as_slice()) {
            (Method::Post, ["api", "rooms", _]) => {
                let settings: Settings = req.json().await?;
                self.create(id, settings)
            }
            (Method::Get, ["api", "rooms", _]) => match self.settings()? {
                Some(settings) => Response::from_json(&RoomInfo {
                    id: id.to_owned(),
                    settings,
                }),
                None => Response::error("no such game", 404),
            },
            (Method::Get, ["api", "socket", _]) => self.accept(&req),
            (Method::Get, ["api", "sgf", _]) => {
                let Some(sgf) = self.with_room(|room| room.sgf())? else {
                    return Response::error("no such game", 404);
                };
                let headers = Headers::new();
                headers.set("Content-Type", "application/x-go-sgf")?;
                headers.set(
                    "Content-Disposition",
                    &format!("attachment; filename=\"{id}.sgf\""),
                )?;
                Ok(Response::ok(sgf)?.with_headers(headers))
            }
            _ => Response::error("not found", 404),
        }
    }

    async fn websocket_message(
        &self,
        ws: WebSocket,
        message: WebSocketIncomingMessage,
    ) -> Result<()> {
        let WebSocketIncomingMessage::String(text) = message else {
            return Ok(());
        };
        let mut me = attachment(&ws);
        match me.token.clone() {
            None => self.hello(&ws, &mut me, &text),
            Some(token) => {
                let Ok(msg) = serde_json::from_str::<ClientMsg>(&text) else {
                    let _ = ws.close(Some(CLOSE_PROTOCOL), Some("malformed message"));
                    return Ok(());
                };
                let result = self.with_room(|room| room.apply(&token, msg))?;
                match result {
                    Some(Ok(outcome)) => self.publish(outcome),
                    Some(Err(reason)) => {
                        let _ = ws.send_with_str(ServerMsg::Rejected { reason }.to_json());
                        Ok(())
                    }
                    None => Ok(()),
                }
            }
        }
    }

    async fn websocket_close(
        &self,
        ws: WebSocket,
        _code: usize,
        _reason: String,
        _was_clean: bool,
    ) -> Result<()> {
        self.leave(&ws)
    }

    async fn websocket_error(&self, ws: WebSocket, _error: Error) -> Result<()> {
        self.leave(&ws)
    }
}

impl Room {
    fn sql(&self) -> SqlStorage {
        self.state.storage().sql()
    }

    fn exists(&self) -> Result<bool> {
        let count: Count = self
            .sql()
            .exec(
                "SELECT count(*) AS n FROM sqlite_master WHERE type = 'table' AND name = 'room'",
                None,
            )?
            .one()?;
        Ok(count.n > 0)
    }

    fn create(&self, id: &str, settings: Settings) -> Result<Response> {
        if let Err(e) = settings.validate() {
            return Response::error(e, 422);
        }
        if self.exists()? {
            return Response::error("room id is taken", 409);
        }
        let sql = self.sql();
        sql.exec(
            "CREATE TABLE room (
                 only       INTEGER PRIMARY KEY CHECK (only = 0),
                 settings   TEXT NOT NULL,
                 tbl        TEXT NOT NULL,
                 updated_at INTEGER NOT NULL
             )",
            None,
        )?;
        sql.exec(
            "CREATE TABLE events (seq INTEGER PRIMARY KEY, event TEXT NOT NULL)",
            None,
        )?;
        sql.exec(
            "INSERT INTO room (only, settings, tbl, updated_at) VALUES (0, ?, ?, ?)",
            vec![
                serde_json::to_string(&settings).map_err(rust_error)?.into(),
                serde_json::to_string(&Table::default())
                    .map_err(rust_error)?
                    .into(),
                now().into(),
            ],
        )?;
        *self.room.borrow_mut() = Some(RoomState::new(settings));
        console_log!("created room {id}");
        Ok(Response::from_json(&RoomInfo {
            id: id.to_owned(),
            settings,
        })?
        .with_status(201))
    }

    fn settings(&self) -> Result<Option<Settings>> {
        self.with_room(|room| room.settings())
    }

    fn with_room<T>(&self, f: impl FnOnce(&mut RoomState) -> T) -> Result<Option<T>> {
        let mut cached = self.room.borrow_mut();
        if cached.is_none() {
            *cached = self.load()?;
        }
        Ok(cached.as_mut().map(f))
    }

    fn load(&self) -> Result<Option<RoomState>> {
        if !self.exists()? {
            return Ok(None);
        }
        let sql = self.sql();
        let rows: Vec<RoomRow> = sql
            .exec("SELECT settings, tbl FROM room", None)?
            .to_array()?;
        let Some(row) = rows.into_iter().next() else {
            return Ok(None);
        };
        let events = sql
            .exec("SELECT event FROM events ORDER BY seq", None)?
            .to_array::<EventRow>()?
            .into_iter()
            .map(|row| serde_json::from_str::<Event>(&row.event))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(rust_error)?;
        let saved = Saved {
            settings: serde_json::from_str(&row.settings).map_err(rust_error)?,
            table: serde_json::from_str(&row.tbl).map_err(rust_error)?,
            events,
        };
        RoomState::restore(saved).map(Some).map_err(rust_error)
    }

    fn accept(&self, req: &Request) -> Result<Response> {
        if req.headers().get("Upgrade")?.as_deref() != Some("websocket") {
            return Response::error("expected a websocket upgrade", 426);
        }
        if !self.exists()? {
            return Response::error("no such game", 404);
        }
        let conn = self
            .state
            .get_websockets()
            .iter()
            .map(|ws| attachment(ws).conn + 1)
            .max()
            .unwrap_or(0);
        let pair = WebSocketPair::new()?;
        self.state.accept_web_socket(&pair.server);
        pair.server.serialize_attachment(Attachment {
            conn,
            ..Attachment::default()
        })?;
        Response::from_websocket(pair.client)
    }

    fn hello(&self, ws: &WebSocket, me: &mut Attachment, text: &str) -> Result<()> {
        let (token, name) = match parse_hello(text) {
            Ok(hello) => hello,
            Err(e) => {
                let _ = ws.close(Some(CLOSE_PROTOCOL), Some(e.to_string()));
                return Ok(());
            }
        };
        let Some(outcome) = self.with_room(|room| room.rename(&token, name))? else {
            let _ = ws.close(Some(CLOSE_PROTOCOL), Some("no such game"));
            return Ok(());
        };

        me.token = Some(token.clone());
        me.you = self.with_room(|room| room.seat_of(&token))?.flatten();
        ws.serialize_attachment(&*me)?;

        let connections = self.connections(None);
        let mut outcome = outcome;
        outcome
            .broadcast
            .insert(0, ServerMsg::Presence { connections });
        self.persist(&outcome)?;
        self.send_all(&outcome.broadcast, None, Some(me.conn));

        let snapshot = self
            .with_room(|room| room.snapshot(&token, connections))?
            .ok_or_else(|| rust_error("room vanished"))?;
        ws.send_with_str(
            ServerMsg::Identity {
                conn: me.conn,
                you: me.you,
            }
            .to_json(),
        )?;
        ws.send_with_str(ServerMsg::Snapshot(Box::new(snapshot)).to_json())?;
        Ok(())
    }

    fn leave(&self, ws: &WebSocket) -> Result<()> {
        let me = attachment(ws);
        let _ = ws.close(None, None::<&str>);
        if me.token.is_none() {
            return Ok(());
        }
        let presence = ServerMsg::Presence {
            connections: self.connections(Some(me.conn)),
        };
        self.send_all(&[presence], Some(me.conn), None);
        Ok(())
    }

    fn connections(&self, except: Option<ConnId>) -> usize {
        self.state
            .get_websockets()
            .iter()
            .map(attachment)
            .filter(|a| a.token.is_some() && Some(a.conn) != except)
            .count()
    }

    fn publish(&self, outcome: Outcome) -> Result<()> {
        self.persist(&outcome)?;
        self.send_all(&outcome.broadcast, None, None);
        Ok(())
    }

    fn persist(&self, outcome: &Outcome) -> Result<()> {
        let sql = self.sql();
        if let Some((seq, event)) = &outcome.logged {
            sql.exec(
                "INSERT INTO events (seq, event) VALUES (?, ?)",
                vec![
                    (*seq as i64).into(),
                    serde_json::to_string(event).map_err(rust_error)?.into(),
                ],
            )?;
        }
        if outcome.table_changed {
            let table = self
                .with_room(|room| room.table())?
                .ok_or_else(|| rust_error("room vanished"))?;
            sql.exec(
                "UPDATE room SET tbl = ? WHERE only = 0",
                vec![serde_json::to_string(&table).map_err(rust_error)?.into()],
            )?;
        }
        if outcome.logged.is_some() || outcome.table_changed {
            sql.exec(
                "UPDATE room SET updated_at = ? WHERE only = 0",
                vec![now().into()],
            )?;
        }
        Ok(())
    }

    fn send_all(&self, msgs: &[ServerMsg], except: Option<ConnId>, skip: Option<ConnId>) {
        for ws in self.state.get_websockets() {
            let mut them = attachment(&ws);
            let Some(token) = them.token.clone() else {
                continue;
            };
            if Some(them.conn) == except || Some(them.conn) == skip {
                continue;
            }
            for msg in msgs {
                if matches!(msg, ServerMsg::Seats { .. }) {
                    let now = self
                        .room
                        .borrow()
                        .as_ref()
                        .and_then(|room| room.seat_of(&token));
                    if now != them.you {
                        them.you = now;
                        let _ = ws.serialize_attachment(&them);
                        let _ = ws.send_with_str(
                            ServerMsg::Identity {
                                conn: them.conn,
                                you: now,
                            }
                            .to_json(),
                        );
                    }
                }
                let _ = ws.send_with_str(msg.to_json());
            }
        }
    }
}

fn now() -> i64 {
    (Date::now().as_millis() / 1000) as i64
}
