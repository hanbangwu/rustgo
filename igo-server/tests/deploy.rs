use std::net::SocketAddr;
use std::time::{Duration, Instant};

use igo_server::{app_with_shutdown, ServerConfig, Shutdown};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

mod common;
use common::*;

const FRONTEND: &str = "https://igo.example";

/// A raw request with extra headers, returning the whole response, headers included.
async fn raw(addr: SocketAddr, method: &str, path: &str, headers: &[(&str, &str)]) -> String {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let headers: String = headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}\r\n"))
        .collect();
    let request =
        format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\n{headers}Connection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    response.to_ascii_lowercase()
}

async fn spawn_for(origins: &[&str]) -> SocketAddr {
    spawn(ServerConfig {
        static_dir: "/nonexistent".into(),
        allowed_origins: origins.iter().map(|o| o.to_string()).collect(),
        ..Default::default()
    })
    .await
}

async fn preflight(addr: SocketAddr, origin: &str) -> String {
    raw(
        addr,
        "OPTIONS",
        "/api/rooms",
        &[
            ("Origin", origin),
            ("Access-Control-Request-Method", "POST"),
            ("Access-Control-Request-Headers", "content-type"),
        ],
    )
    .await
}

#[tokio::test]
async fn heartbeats_are_accepted_silently() {
    let addr = spawn_default().await;
    let room = new_room(addr).await;
    let mut client = Client::join(addr, &room, "tok-a", "Ada").await;

    client.send(&json!({ "type": "heartbeat" })).await;
    client.expect_silence().await;
}

#[tokio::test]
async fn an_allowed_origin_may_create_rooms() {
    let addr = spawn_for(&[FRONTEND]).await;

    let response = preflight(addr, FRONTEND).await;
    assert!(
        response.contains(&format!("access-control-allow-origin: {FRONTEND}")),
        "{response}"
    );
    assert!(response.contains("content-type"), "{response}");
}

#[tokio::test]
async fn other_origins_are_not_allowed() {
    let addr = spawn_for(&[FRONTEND]).await;
    let response = preflight(addr, "https://elsewhere.example").await;
    assert!(
        !response.contains("access-control-allow-origin"),
        "{response}"
    );
}

#[tokio::test]
async fn no_cors_headers_without_allowed_origins() {
    let addr = spawn_default().await;
    let response = raw(addr, "GET", "/api/health", &[("Origin", FRONTEND)]).await;
    assert!(
        !response.contains("access-control-allow-origin"),
        "{response}"
    );
}

#[tokio::test]
async fn shutdown_closes_open_games() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let shutdown = Shutdown::new();
    let config = ServerConfig {
        static_dir: "/nonexistent".into(),
        ..Default::default()
    };
    let router = app_with_shutdown(config, shutdown.clone()).unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let room = new_room(addr).await;
    let mut client = Client::join(addr, &room, "tok-a", "Ada").await;

    shutdown.trigger();
    client.expect_closed().await;

    let started = Instant::now();
    shutdown.drained(Duration::from_secs(5)).await;
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "drained promptly"
    );
}
