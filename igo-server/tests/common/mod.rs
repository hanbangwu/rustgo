#![allow(dead_code)]

use std::net::SocketAddr;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use igo_server::{app, ServerConfig};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

const RECV_TIMEOUT: Duration = Duration::from_secs(5);

pub async fn spawn(config: ServerConfig) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(config).expect("app starts"))
            .await
            .unwrap();
    });
    addr
}

pub async fn spawn_default() -> SocketAddr {
    spawn(ServerConfig {
        static_dir: "/nonexistent".into(),
        ..Default::default()
    })
    .await
}

pub struct Client {
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
    moves: Vec<Value>,
    phase: Value,
    to_play: Value,
    captures: Value,
}

impl Client {
    pub async fn connect(addr: SocketAddr, room: &str) -> Client {
        let (ws, _) = connect_async(format!("ws://{addr}/api/socket/{room}"))
            .await
            .expect("websocket handshake");
        Client {
            ws,
            moves: Vec::new(),
            phase: Value::Null,
            to_play: Value::Null,
            captures: Value::Null,
        }
    }

    pub async fn join(addr: SocketAddr, room: &str, token: &str, name: &str) -> Client {
        let mut client = Client::connect(addr, room).await;
        client
            .send(&json!({ "type": "hello", "token": token, "name": name }))
            .await;
        client.expect("identity").await;
        client.expect("snapshot").await;
        client
    }

    pub async fn send(&mut self, msg: &Value) {
        self.ws
            .send(Message::text(msg.to_string()))
            .await
            .expect("send");
    }

    pub async fn recv(&mut self) -> Value {
        loop {
            let msg = tokio::time::timeout(RECV_TIMEOUT, self.ws.next())
                .await
                .expect("timed out waiting for a message")
                .expect("socket closed unexpectedly")
                .expect("websocket error");
            match msg {
                Message::Text(text) => {
                    let value: Value = serde_json::from_str(&text).expect("valid JSON");
                    self.absorb(&value);
                    return value;
                }
                Message::Ping(_) | Message::Pong(_) => continue,
                Message::Close(_) => panic!("server closed the connection"),
                other => panic!("unexpected frame: {other:?}"),
            }
        }
    }

    fn absorb(&mut self, msg: &Value) {
        match msg["type"].as_str() {
            Some("snapshot") => {
                let n = msg["move_number"].as_u64().unwrap_or(0) as usize;
                self.moves.resize(n, Value::Null);
                self.phase = msg["phase"].clone();
                self.to_play = msg["to_play"].clone();
                self.captures = msg["captures"].clone();
            }
            Some("moved") => {
                let n = msg["applied"]["move_number"].as_u64().unwrap() as usize;
                assert_eq!(n, self.moves.len(), "moves arrive in order: {msg}");
                self.moves.push(msg["applied"].clone());
                self.phase = msg["phase"].clone();
                self.to_play = msg["to_play"].clone();
                self.captures = msg["captures"].clone();
            }
            Some("phase_changed") => self.phase = msg["phase"].clone(),
            _ => {}
        }
    }

    pub async fn expect(&mut self, ty: &str) -> Value {
        self.expect_matching(ty, |_| true).await
    }

    pub async fn expect_matching(
        &mut self,
        ty: &str,
        mut predicate: impl FnMut(&Value) -> bool,
    ) -> Value {
        for _ in 0..32 {
            let msg = self.recv().await;
            if msg["type"] == ty && predicate(&msg) {
                return msg;
            }
        }
        panic!("never received a matching {ty:?} message");
    }

    pub async fn expect_seats(&mut self, black: Option<&str>, white: Option<&str>) -> Value {
        self.expect_matching("seats", |v| {
            v["seats"]["black"] == json!(black) && v["seats"]["white"] == json!(white)
        })
        .await
    }

    pub async fn expect_move(&mut self, n: usize) -> Value {
        while self.moves.len() <= n {
            self.recv().await;
        }
        self.moves[n].clone()
    }

    pub async fn sync_to(&mut self, n: usize) {
        while self.moves.len() < n {
            self.recv().await;
        }
    }

    pub fn phase(&self) -> &Value {
        &self.phase
    }

    pub fn to_play(&self) -> &Value {
        &self.to_play
    }

    pub fn captures(&self) -> &Value {
        &self.captures
    }

    pub fn move_count(&self) -> usize {
        self.moves.len()
    }

    pub async fn expect_silence(&mut self) {
        let result = tokio::time::timeout(Duration::from_millis(250), self.ws.next()).await;
        if let Ok(Some(Ok(Message::Text(text)))) = result {
            panic!("expected no message, got {text}");
        }
    }

    pub async fn expect_closed(&mut self) {
        for _ in 0..32 {
            let msg = tokio::time::timeout(RECV_TIMEOUT, self.ws.next())
                .await
                .expect("timed out waiting for close");
            match msg {
                None | Some(Ok(Message::Close(_))) | Some(Err(_)) => return,
                Some(Ok(_)) => continue,
            }
        }
        panic!("connection stayed open");
    }
}

/// Creates a room over HTTP and returns its id.
pub async fn new_room(addr: SocketAddr) -> String {
    let (status, body) = http("POST", &format!("http://{addr}/api/rooms"), Some("{}")).await;
    assert_eq!(status, 201, "{body}");
    let created: Value = serde_json::from_str(&body).expect("JSON body");
    created["id"].as_str().expect("an id").to_string()
}

/// A bare-bones HTTP/1.1 request, returning the status and body.
pub async fn http(method: &str, url: &str, json_body: Option<&str>) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let rest = url.strip_prefix("http://").expect("http url");
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let mut stream = TcpStream::connect(authority).await.unwrap();
    let body = json_body.unwrap_or("");
    let headers = if json_body.is_some() {
        format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        )
    } else {
        String::new()
    };
    let request = format!(
        "{method} /{path} HTTP/1.1\r\nHost: {authority}\r\n{headers}Connection: close\r\n\r\n{body}"
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();

    let status = response
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or_default();
    (status, body)
}

pub async fn http_body(url: &str) -> String {
    http("GET", url, None).await.1
}

pub async fn http_status(url: &str) -> u16 {
    http("GET", url, None).await.0
}

pub fn at(row: u16, col: u16) -> u16 {
    row * 19 + col
}

pub fn play(move_number: usize, vertex: u16) -> Value {
    json!({
        "type": "play",
        "move_number": move_number,
        "move": { "type": "play", "vertex": vertex },
    })
}

pub fn pass(move_number: usize) -> Value {
    json!({
        "type": "play",
        "move_number": move_number,
        "move": { "type": "pass" },
    })
}

pub fn claim(color: &str) -> Value {
    json!({ "type": "claim_seat", "color": color })
}
