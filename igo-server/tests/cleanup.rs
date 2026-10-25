use std::time::Duration;

use igo_server::ServerConfig;
use serde_json::json;

mod common;
use common::*;

async fn spawn_impatient() -> std::net::SocketAddr {
    spawn(ServerConfig {
        max_idle: Duration::from_millis(50),
        sweep_interval: Duration::from_millis(25),
        static_dir: "/nonexistent".into(),
        ..Default::default()
    })
    .await
}

#[tokio::test]
async fn idle_rooms_are_reclaimed() {
    let addr = spawn_impatient().await;
    let room = new_room(addr).await;

    let client = Client::join(addr, &room, "tok-a", "Ada").await;
    assert_eq!(room_count(addr).await, 1);
    drop(client);

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(room_count(addr).await, 0, "the room should have been swept");
    assert_eq!(
        http_status(&format!("http://{addr}/api/rooms/{room}")).await,
        404
    );
}

#[tokio::test]
async fn a_room_that_is_never_joined_is_reclaimed() {
    let addr = spawn_impatient().await;
    new_room(addr).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(room_count(addr).await, 0);
}

#[tokio::test]
async fn a_game_with_players_connected_is_never_swept() {
    let addr = spawn_impatient().await;
    let room = new_room(addr).await;

    let mut black = Client::join(addr, &room, "tok-b", "Black").await;
    let mut white = Client::join(addr, &room, "tok-w", "White").await;
    black.send(&claim("black")).await;
    white.send(&claim("white")).await;
    black.expect_seats(Some("Black"), Some("White")).await;

    // Far longer than max_idle, with nobody new arriving.
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(room_count(addr).await, 1);

    black.send(&play(0, at(3, 3))).await;
    let applied = white.expect_move(0).await;
    assert_eq!(applied["move"]["vertex"], at(3, 3));
}

#[tokio::test]
async fn stats_reports_live_rooms() {
    let addr = spawn_default().await;
    assert_eq!(room_count(addr).await, 0);

    new_room(addr).await;
    new_room(addr).await;

    assert_eq!(room_count(addr).await, 2);
}

#[tokio::test]
async fn room_creation_is_capped() {
    let addr = spawn(ServerConfig {
        max_rooms: 1,
        static_dir: "/nonexistent".into(),
        ..Default::default()
    })
    .await;
    new_room(addr).await;
    let (status, _) = http("POST", &format!("http://{addr}/api/rooms"), Some("{}")).await;
    assert_eq!(status, 503);
}

async fn room_count(addr: std::net::SocketAddr) -> u64 {
    let body = http_body(&format!("http://{addr}/api/stats")).await;
    let stats: serde_json::Value = serde_json::from_str(&body).unwrap_or(json!({}));
    stats["rooms"].as_u64().unwrap_or(u64::MAX)
}
