use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::board::{Board, Color, Point, Vertex};
use crate::score::{self, Score};
use crate::zobrist;

pub const DEFAULT_KOMI: f32 = 7.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Move {
    Play { vertex: Vertex },
    Pass,
    Resign,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum Phase {
    Playing,
    Scoring,
    Finished { result: GameResult },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum GameResult {
    Counted { winner: Option<Color>, score: Score },
    Resignation { winner: Color },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Applied {
    pub move_number: usize,
    pub color: Color,
    #[serde(rename = "move")]
    pub mv: Move,
    pub captured: Vec<Vertex>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum MoveError {
    #[error("the game is not in progress")]
    NotPlaying,
    #[error("it is not your turn")]
    WrongTurn,
    #[error("that point is not on the board")]
    OffBoard,
    #[error("that point is already occupied")]
    Occupied,
    #[error("that move is suicide")]
    Suicide,
    #[error("that move repeats a previous position (ko)")]
    Superko,
}

#[derive(Debug, Clone)]
pub struct Game {
    board: Board,
    to_play: Color,
    history: Vec<Applied>,
    position_hashes: HashSet<u64>,
    hash: u64,
    captures: [u32; 2],
    consecutive_passes: u8,
    phase: Phase,
    komi: f32,
    setup: [Vec<Vertex>; 2],
}

impl Game {
    pub fn new(size: u8, komi: f32) -> Game {
        Game::from_board(Board::new(size), Color::Black, komi)
    }

    pub fn with_handicap(size: u8, komi: f32, stones: u8) -> Option<Game> {
        let mut board = Board::new(size);
        let points = handicap_points(board.size(), stones)?;
        for &v in &points {
            board.set(v, Point::Stone(Color::Black));
        }
        let to_play = if points.is_empty() {
            Color::Black
        } else {
            Color::White
        };
        Some(Game::from_board(board, to_play, komi))
    }

    pub fn from_board(board: Board, to_play: Color, komi: f32) -> Game {
        let hash = zobrist::hash_board(&board);
        let mut setup = [Vec::new(), Vec::new()];
        for v in board.vertices() {
            if let Point::Stone(c) = board.get(v) {
                setup[c.index()].push(v);
            }
        }
        Game {
            setup,
            board,
            to_play,
            history: Vec::new(),
            position_hashes: HashSet::from([hash]),
            hash,
            captures: [0; 2],
            consecutive_passes: 0,
            phase: Phase::Playing,
            komi,
        }
    }

    pub fn board(&self) -> &Board {
        &self.board
    }

    pub fn to_play(&self) -> Color {
        self.to_play
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    pub fn komi(&self) -> f32 {
        self.komi
    }

    pub fn setup(&self) -> &[Vec<Vertex>; 2] {
        &self.setup
    }

    pub fn history(&self) -> &[Applied] {
        &self.history
    }

    pub fn move_number(&self) -> usize {
        self.history.len()
    }

    pub fn captures(&self) -> [u32; 2] {
        self.captures
    }

    pub fn is_finished(&self) -> bool {
        matches!(self.phase, Phase::Finished { .. })
    }

    pub fn play(&mut self, color: Color, mv: Move) -> Result<Applied, MoveError> {
        if !matches!(self.phase, Phase::Playing) {
            return Err(MoveError::NotPlaying);
        }
        if color != self.to_play {
            return Err(MoveError::WrongTurn);
        }

        match mv {
            Move::Resign => {
                self.phase = Phase::Finished {
                    result: GameResult::Resignation {
                        winner: color.opposite(),
                    },
                };
                Ok(self.record(color, mv, Vec::new()))
            }
            Move::Pass => {
                self.consecutive_passes += 1;
                if self.consecutive_passes >= 2 {
                    self.phase = Phase::Scoring;
                }
                self.to_play = color.opposite();
                Ok(self.record(color, mv, Vec::new()))
            }
            Move::Play { vertex } => self.play_stone(color, vertex),
        }
    }

    fn play_stone(&mut self, color: Color, vertex: Vertex) -> Result<Applied, MoveError> {
        if !self.board.contains(vertex) {
            return Err(MoveError::OffBoard);
        }
        if self.board.get(vertex) != Point::Empty {
            return Err(MoveError::Occupied);
        }

        let mut next = self.board.clone();
        next.set(vertex, Point::Stone(color));

        let mut captured = Vec::new();
        for n in next.neighbors(vertex) {
            if next.get(n) == Point::Stone(color.opposite()) && !next.has_liberty(n) {
                let group = next.group_at(n).expect("neighbor is a stone");
                for &s in &group.stones {
                    next.set(s, Point::Empty);
                }
                captured.extend_from_slice(&group.stones);
            }
        }

        if !next.has_liberty(vertex) {
            return Err(MoveError::Suicide);
        }

        let mut next_hash = self.hash ^ zobrist::stone(vertex, color);
        for &s in &captured {
            next_hash ^= zobrist::stone(s, color.opposite());
        }
        if self.position_hashes.contains(&next_hash) {
            return Err(MoveError::Superko);
        }

        self.board = next;
        self.hash = next_hash;
        self.position_hashes.insert(next_hash);
        self.captures[color.index()] += captured.len() as u32;
        self.consecutive_passes = 0;
        self.to_play = color.opposite();
        Ok(self.record(color, Move::Play { vertex }, captured))
    }

    pub fn replay_move(&mut self, color: Color, mv: Move) -> Result<Applied, MoveError> {
        self.to_play = color;
        self.play(color, mv)
    }

    fn record(&mut self, color: Color, mv: Move, captured: Vec<Vertex>) -> Applied {
        let applied = Applied {
            move_number: self.history.len(),
            color,
            mv,
            captured,
        };
        self.history.push(applied.clone());
        applied
    }

    pub fn finish_with_dead(&mut self, dead: &HashSet<Vertex>) -> Result<Score, MoveError> {
        if !matches!(self.phase, Phase::Scoring) {
            return Err(MoveError::NotPlaying);
        }
        let score = score::area_score(&self.board, dead, self.komi);
        self.phase = Phase::Finished {
            result: GameResult::Counted {
                winner: score.winner(),
                score: score.clone(),
            },
        };
        Ok(score)
    }

    pub fn provisional_score(&self, dead: &HashSet<Vertex>) -> Score {
        score::area_score(&self.board, dead, self.komi)
    }

    pub fn resume_play(&mut self) -> Result<(), MoveError> {
        if !matches!(self.phase, Phase::Scoring) {
            return Err(MoveError::NotPlaying);
        }
        self.phase = Phase::Playing;
        self.consecutive_passes = 0;
        Ok(())
    }
}

pub fn handicap_points(size: u8, stones: u8) -> Option<Vec<Vertex>> {
    if stones == 0 {
        return Some(Vec::new());
    }
    if stones == 1 || stones > 9 || size < 7 || (stones > 4 && size.is_multiple_of(2)) {
        return None;
    }

    let edge = if size < 13 { 2 } else { 3 };
    let (near, mid, far) = (edge, size / 2, size - 1 - edge);
    let at = |row: u8, col: u8| row as Vertex * size as Vertex + col as Vertex;

    let mut points = vec![at(near, far), at(far, near)];
    if stones >= 3 {
        points.push(at(far, far));
    }
    if stones >= 4 {
        points.push(at(near, near));
    }
    if stones >= 6 {
        points.push(at(mid, near));
        points.push(at(mid, far));
    }
    if stones >= 8 {
        points.push(at(near, mid));
        points.push(at(far, mid));
    }
    if stones % 2 == 1 && stones >= 5 {
        points.push(at(mid, mid));
    }
    Some(points)
}
