use std::collections::{HashMap, HashSet};

use anyhow::{anyhow, bail, Context, Result};
use igo_core::{Color, Game, Phase, Vertex};

use crate::protocol::{ClientMsg, PlayerToken, Seats, ServerMsg, Settings, Snapshot};
use crate::record::{Event, Saved, SeatRecord, Table};

const MAX_NAME_LEN: usize = 40;
const MAX_TOKEN_LEN: usize = 64;

#[derive(Debug, Default)]
pub struct Outcome {
    pub broadcast: Vec<ServerMsg>,
    pub logged: Option<(usize, Event)>,
    pub table_changed: bool,
}

impl Outcome {
    fn send(msg: ServerMsg) -> Outcome {
        Outcome {
            broadcast: vec![msg],
            ..Outcome::default()
        }
    }
}

pub fn parse_hello(text: &str) -> Result<(PlayerToken, String)> {
    let msg: ClientMsg = serde_json::from_str(text).context("malformed hello")?;
    let ClientMsg::Hello { token, name } = msg else {
        bail!("first message must be hello");
    };
    if token.is_empty() || token.len() > MAX_TOKEN_LEN {
        bail!("invalid player token");
    }
    Ok((token, name))
}

pub struct RoomState {
    settings: Settings,
    game: Game,
    seats: [Option<SeatRecord>; 2],
    names: HashMap<PlayerToken, String>,
    dead: HashSet<Vertex>,
    accepted: HashSet<Color>,
    logged: usize,
}

impl RoomState {
    pub fn new(settings: Settings) -> RoomState {
        let game = Game::with_handicap(settings.size, settings.komi, settings.handicap)
            .expect("settings were validated");
        RoomState {
            settings,
            game,
            seats: [None, None],
            names: HashMap::new(),
            dead: HashSet::new(),
            accepted: HashSet::new(),
            logged: 0,
        }
    }

    pub fn restore(saved: Saved) -> Result<RoomState> {
        saved.settings.validate().map_err(|e| anyhow!(e))?;
        let mut state = RoomState::new(saved.settings);
        for (seq, event) in saved.events.iter().enumerate() {
            state
                .apply_event(event)
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
        Ok(state)
    }

    pub fn settings(&self) -> Settings {
        self.settings
    }

    pub fn seat_of(&self, token: &str) -> Option<Color> {
        [Color::Black, Color::White].into_iter().find(|color| {
            self.seats[color.index()]
                .as_ref()
                .is_some_and(|holder| holder.token == token)
        })
    }

    pub fn sgf(&self) -> String {
        igo_core::sgf::to_sgf(&self.game)
    }

    pub fn table(&self) -> Table {
        Table {
            seats: self.seats.clone(),
            dead: self.dead_sorted(),
            accepted: self.accepted_sorted(),
        }
    }

    pub fn snapshot(&self, token: &str, connections: usize) -> Snapshot {
        Snapshot {
            size: self.settings.size,
            komi: self.settings.komi,
            handicap: self.settings.handicap,
            board: self.game.board().as_bytes(),
            to_play: self.game.to_play(),
            move_number: self.game.move_number(),
            captures: self.game.captures(),
            phase: self.game.phase().clone(),
            seats: self.seats_view(),
            dead: self.dead_sorted(),
            accepted: self.accepted_sorted(),
            you: self.seat_of(token),
            connections,
        }
    }

    fn seats_view(&self) -> Seats {
        let name = |c: Color| self.seats[c.index()].as_ref().map(|h| h.name.clone());
        Seats {
            black: name(Color::Black),
            white: name(Color::White),
        }
    }

    fn seats_message(&self) -> ServerMsg {
        ServerMsg::Seats {
            seats: self.seats_view(),
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

    fn apply_event(&mut self, event: &Event) -> Result<()> {
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

    fn log(&mut self, event: Event) -> Option<(usize, Event)> {
        let seq = self.logged;
        self.logged += 1;
        Some((seq, event))
    }

    pub fn rename(&mut self, token: &str, name: String) -> Outcome {
        let name: String = name.chars().take(MAX_NAME_LEN).collect();
        let name = if name.trim().is_empty() {
            "Anonymous".to_string()
        } else {
            name
        };

        self.names.insert(token.to_owned(), name.clone());
        let Some(color) = self.seat_of(token) else {
            return Outcome::default();
        };
        let Some(holder) = self.seats[color.index()].as_mut() else {
            return Outcome::default();
        };
        if holder.name == name {
            return Outcome::default();
        }
        holder.name = name;
        Outcome {
            table_changed: true,
            ..Outcome::send(self.seats_message())
        }
    }

    pub fn apply(&mut self, token: &str, msg: ClientMsg) -> Result<Outcome, String> {
        match msg {
            ClientMsg::Hello {
                token: sent,
                name: new_name,
            } => {
                if sent != token {
                    return Err("cannot change identity on an open connection".into());
                }
                Ok(self.rename(token, new_name))
            }

            ClientMsg::ClaimSeat { color } => {
                if let Some(held) = self.seat_of(token) {
                    return Err(if held == color {
                        "you already have that seat".into()
                    } else {
                        "you are already playing the other colour".into()
                    });
                }
                if self.seats[color.index()].is_some() {
                    return Err("that seat is taken".into());
                }
                let name = self
                    .names
                    .get(token)
                    .cloned()
                    .unwrap_or_else(|| "Anonymous".into());
                self.seats[color.index()] = Some(SeatRecord {
                    token: token.to_owned(),
                    name,
                });
                Ok(Outcome {
                    table_changed: true,
                    ..Outcome::send(self.seats_message())
                })
            }

            ClientMsg::ReleaseSeat => {
                let color = self.seat_of(token).ok_or("you do not have a seat")?;
                if self.game.move_number() > 0 {
                    return Err("the game has already started".into());
                }
                self.seats[color.index()] = None;
                Ok(Outcome {
                    table_changed: true,
                    ..Outcome::send(self.seats_message())
                })
            }

            ClientMsg::Play { move_number, mv } => {
                let color = self
                    .seat_of(token)
                    .ok_or("you are watching this game, not playing it")?;

                let current = self.game.move_number();
                if move_number != current {
                    return Err(format!(
                        "stale move: sent at move {move_number}, game is at {current}"
                    ));
                }

                let applied = self.game.play(color, mv).map_err(|e| e.to_string())?;
                let logged = self.log(Event::Move { color, mv });

                let mut broadcast = vec![ServerMsg::Moved {
                    applied,
                    to_play: self.game.to_play(),
                    captures: self.game.captures(),
                    phase: self.game.phase().clone(),
                }];
                if matches!(self.game.phase(), Phase::Scoring) {
                    broadcast.push(self.dead_message());
                }
                Ok(Outcome {
                    broadcast,
                    logged,
                    table_changed: false,
                })
            }

            ClientMsg::ToggleDead { vertex } => {
                if !matches!(self.game.phase(), Phase::Scoring) {
                    return Err("the game is not being scored".into());
                }
                self.seat_of(token)
                    .ok_or("only the players may mark stones")?;

                let group = self
                    .game
                    .board()
                    .group_at(vertex)
                    .ok_or("there is no stone there")?;

                let all_dead = group.stones.iter().all(|v| self.dead.contains(v));
                for v in &group.stones {
                    if all_dead {
                        self.dead.remove(v);
                    } else {
                        self.dead.insert(*v);
                    }
                }
                self.accepted.clear();
                Ok(Outcome {
                    table_changed: true,
                    ..Outcome::send(self.dead_message())
                })
            }

            ClientMsg::AcceptScore => {
                if !matches!(self.game.phase(), Phase::Scoring) {
                    return Err("the game is not being scored".into());
                }
                let color = self
                    .seat_of(token)
                    .ok_or("only the players may agree the score")?;
                if self.seats.iter().any(Option::is_none) {
                    return Err("both seats must be filled to finish the game".into());
                }

                self.accepted.insert(color);
                if self.accepted.len() < 2 {
                    return Ok(Outcome {
                        table_changed: true,
                        ..Outcome::send(self.dead_message())
                    });
                }

                let dead = self.dead_sorted();
                self.game
                    .finish_with_dead(&self.dead)
                    .map_err(|e| e.to_string())?;
                let logged = self.log(Event::Finish { dead });
                Ok(Outcome {
                    broadcast: vec![
                        self.dead_message(),
                        ServerMsg::PhaseChanged {
                            phase: self.game.phase().clone(),
                        },
                    ],
                    logged,
                    table_changed: true,
                })
            }

            ClientMsg::Heartbeat => Ok(Outcome::default()),

            ClientMsg::ResumePlay => {
                self.seat_of(token)
                    .ok_or("only the players may resume the game")?;
                self.apply_event(&Event::Resume)
                    .map_err(|_| "the game is not being scored".to_string())?;
                let logged = self.log(Event::Resume);
                Ok(Outcome {
                    broadcast: vec![
                        ServerMsg::PhaseChanged {
                            phase: self.game.phase().clone(),
                        },
                        self.dead_message(),
                    ],
                    logged,
                    table_changed: true,
                })
            }
        }
    }
}
