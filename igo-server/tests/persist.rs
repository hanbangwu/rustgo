use std::path::Path;
use std::time::Duration;

use igo_server::ServerConfig;
use serde_json::json;

mod common;
use common::*;

async fn spawn_with(db: &Path) -> std::net::SocketAddr {
    spawn(ServerConfig {
        database: Some(db.to_path_buf()),
        static_dir: "/nonexistent".into(),
        ..Default::default()
    })
    .await
}

#[tokio::test]
async fn a_game_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("igo.db");

    let first = spawn_with(&db).await;
    let (_, body) = http(
        "POST",
        &format!("http://{first}/api/rooms"),
        Some(r#"{"size":13,"komi":6.5}"#),
    )
    .await;
    let room = serde_json::from_str::<serde_json::Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let mut black = Client::join(first, &room, "tok-b", "Black").await;
    let mut white = Client::join(first, &room, "tok-w", "White").await;
    black.send(&claim("black")).await;
    white.send(&claim("white")).await;
    black.expect_seats(Some("Black"), Some("White")).await;

    // Play, enter scoring, resume, play on, enter scoring again and mark a stone.
    let script = [play(0, 0), play(1, 1), pass(2), pass(3)];
    for (n, msg) in script.iter().enumerate() {
        let mover = if n % 2 == 0 { &mut black } else { &mut white };
        mover.send(msg).await;
        black.expect_move(n).await;
        white.expect_move(n).await;
    }
    black.send(&json!({ "type": "resume_play" })).await;
    white.expect("phase_changed").await;
    black.send(&play(4, 20)).await;
    white.expect_move(4).await;
    white.send(&pass(5)).await;
    black.expect_move(5).await;
    black.send(&pass(6)).await;
    white.expect_move(6).await;
    white
        .send(&json!({ "type": "toggle_dead", "vertex": 1 }))
        .await;
    black
        .expect_matching("dead", |v| v["vertices"] == json!([1]))
        .await;
    let before = http_body(&format!("http://{first}/api/sgf/{room}")).await;

    // A second server on the same file stands in for a restart.
    let second = spawn_with(&db).await;
    let (status, body) = http("GET", &format!("http://{second}/api/rooms/{room}"), None).await;
    assert_eq!(status, 200, "{body}");
    assert!(
        body.contains(r#""size":13"#) && body.contains(r#""komi":6.5"#),
        "{body}"
    );

    let mut back = Client::connect(second, &room).await;
    back.send(&json!({ "type": "hello", "token": "tok-b", "name": "Black" }))
        .await;
    assert_eq!(back.expect("identity").await["you"], "black");
    let snapshot = back.expect("snapshot").await;
    assert_eq!(snapshot["move_number"], 7);
    assert_eq!(snapshot["phase"]["state"], "scoring");
    assert_eq!(
        snapshot["seats"],
        json!({ "black": "Black", "white": "White" })
    );
    assert_eq!(snapshot["dead"], json!([1]));
    assert_eq!(snapshot["board"][20], 1);

    let after = http_body(&format!("http://{second}/api/sgf/{room}")).await;
    assert_eq!(after, before, "the record is identical");
}

#[tokio::test]
async fn expired_games_are_deleted_from_the_database() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("igo.db");
    let config = ServerConfig {
        database: Some(db.clone()),
        max_idle: Duration::from_millis(50),
        sweep_interval: Duration::from_millis(25),
        static_dir: "/nonexistent".into(),
        ..Default::default()
    };

    let first = spawn(config.clone()).await;
    let room = new_room(first).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let second = spawn_with(&db).await;
    assert_eq!(
        http_status(&format!("http://{second}/api/rooms/{room}")).await,
        404
    );
}
