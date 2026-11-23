use rustgo_core::{Applied, Color, Move, Phase, Score, Vertex, DEFAULT_KOMI};
use serde::{Deserialize, Serialize};

pub type ConnId = u64;

pub type PlayerToken = String;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Seats {
    pub black: Option<String>,
    pub white: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub size: u8,
    pub komi: f32,
    pub handicap: u8,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            size: 19,
            komi: DEFAULT_KOMI,
            handicap: 0,
        }
    }
}

impl Settings {
    pub const SIZES: [u8; 3] = [9, 13, 19];

    pub fn validate(&self) -> Result<(), &'static str> {
        if !Self::SIZES.contains(&self.size) {
            return Err("board size must be 9, 13 or 19");
        }
        if !self.komi.is_finite() || self.komi.abs() > 100.0 || (self.komi * 2.0).fract() != 0.0 {
            return Err("komi must be a multiple of 0.5 between -100 and 100");
        }
        if rustgo_core::handicap_points(self.size, self.handicap).is_none() {
            return Err("handicap must be 0 or between 2 and 9 stones");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomInfo {
    pub id: String,
    #[serde(flatten)]
    pub settings: Settings,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewRoom {
    #[serde(default = "default_size")]
    pub size: u8,
    pub komi: Option<f32>,
    #[serde(default)]
    pub handicap: u8,
}

fn default_size() -> u8 {
    19
}

impl From<NewRoom> for Settings {
    fn from(req: NewRoom) -> Settings {
        let komi = req
            .komi
            .unwrap_or(if req.handicap > 0 { 0.5 } else { DEFAULT_KOMI });
        Settings {
            size: req.size,
            komi,
            handicap: req.handicap,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    Hello {
        token: PlayerToken,
        name: String,
    },
    ClaimSeat {
        color: Color,
    },
    ReleaseSeat,
    Play {
        move_number: usize,
        #[serde(rename = "move")]
        mv: Move,
    },
    ToggleDead {
        vertex: Vertex,
    },
    AcceptScore,
    ResumePlay,
    Heartbeat,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    Identity {
        conn: ConnId,
        you: Option<Color>,
    },
    Snapshot(Box<Snapshot>),
    Moved {
        applied: Applied,
        to_play: Color,
        captures: [u32; 2],
        phase: Phase,
    },
    Seats {
        seats: Seats,
    },
    Dead {
        vertices: Vec<Vertex>,
        accepted: Vec<Color>,
        score: Score,
        territory: Vec<u8>,
    },
    PhaseChanged {
        phase: Phase,
    },
    Presence {
        connections: usize,
    },
    Rejected {
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub size: u8,
    pub komi: f32,
    pub handicap: u8,
    pub board: Vec<u8>,
    pub to_play: Color,
    pub move_number: usize,
    pub captures: [u32; 2],
    pub phase: Phase,
    pub seats: Seats,
    pub dead: Vec<Vertex>,
    pub accepted: Vec<Color>,
    pub you: Option<Color>,
    pub connections: usize,
}

impl ServerMsg {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("ServerMsg is always serializable")
    }
}
