use rustgo_core::{Color, Move, Vertex};
use serde::{Deserialize, Serialize};

use crate::protocol::Settings;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Move {
        color: Color,
        #[serde(rename = "move")]
        mv: Move,
    },
    Resume,
    Finish {
        dead: Vec<Vertex>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeatRecord {
    pub token: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Table {
    pub seats: [Option<SeatRecord>; 2],
    pub dead: Vec<Vertex>,
    pub accepted: Vec<Color>,
}

pub struct Saved {
    pub settings: Settings,
    pub table: Table,
    pub events: Vec<Event>,
}
