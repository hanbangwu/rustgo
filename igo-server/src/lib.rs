mod id;
mod protocol;
mod record;
mod room;
mod state;

use id::{random_room_id, valid_room_id};
use protocol::{NewRoom, Settings};
use worker::wasm_bindgen::JsValue;
use worker::*;

pub use room::Room;

const ROOMS: &str = "ROOMS";

const MAX_CREATE_ATTEMPTS: usize = 8;

#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let origin = allowed_origin(&req, &env)?;
    if req.method() == Method::Options {
        return preflight(origin.as_deref());
    }
    let response = route(req, &env).await?;
    match origin {
        Some(origin) => with_cors(response, &origin),
        None => Ok(response),
    }
}

async fn route(req: Request, env: &Env) -> Result<Response> {
    let path = req.path();
    let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
    match (req.method(), segments.as_slice()) {
        (Method::Get, ["api", "health"]) => Response::ok("ok"),
        (Method::Post, ["api", "rooms"]) => create(req, env).await,
        (Method::Get, ["api", "rooms" | "socket" | "sgf", id]) => {
            if !valid_room_id(id) {
                return Response::error("invalid room id", 400);
            }
            env.durable_object(ROOMS)?
                .get_by_name(id)?
                .fetch_with_request(req)
                .await
        }
        _ => Response::error("not found", 404),
    }
}

async fn create(mut req: Request, env: &Env) -> Result<Response> {
    let Ok(new_room) = req.json::<NewRoom>().await else {
        return Response::error("expected a JSON body", 400);
    };
    let settings = Settings::from(new_room);
    if let Err(e) = settings.validate() {
        return Response::error(e, 422);
    }
    let body = serde_json::to_string(&settings).map_err(|e| Error::RustError(e.to_string()))?;
    let rooms = env.durable_object(ROOMS)?;

    for _ in 0..MAX_CREATE_ATTEMPTS {
        let id = new_room_id()?;
        let headers = Headers::new();
        headers.set("Content-Type", "application/json")?;
        let mut init = RequestInit::new();
        init.with_method(Method::Post)
            .with_headers(headers)
            .with_body(Some(JsValue::from_str(&body)));
        let inner = Request::new_with_init(&format!("https://room/api/rooms/{id}"), &init)?;
        let response = rooms.get_by_name(&id)?.fetch_with_request(inner).await?;
        if response.status_code() != 409 {
            return Ok(response);
        }
    }
    Response::error("could not find a free room id", 500)
}

fn allowed_origin(req: &Request, env: &Env) -> Result<Option<String>> {
    let Some(origin) = req.headers().get("Origin")? else {
        return Ok(None);
    };
    let allowed = env
        .var("ALLOWED_ORIGINS")
        .map(|v| v.to_string())
        .unwrap_or_default();
    let ok = allowed
        .split(',')
        .map(|o| o.trim().trim_end_matches('/'))
        .any(|o| !o.is_empty() && o == origin);
    Ok(ok.then_some(origin))
}

fn preflight(origin: Option<&str>) -> Result<Response> {
    let headers = Headers::new();
    if let Some(origin) = origin {
        headers.set("Access-Control-Allow-Origin", origin)?;
        headers.set("Access-Control-Allow-Methods", "GET, POST")?;
        headers.set("Access-Control-Allow-Headers", "Content-Type")?;
        headers.set("Access-Control-Max-Age", "86400")?;
    }
    headers.set("Vary", "Origin")?;
    Ok(Response::empty()?.with_status(204).with_headers(headers))
}

fn with_cors(response: Response, origin: &str) -> Result<Response> {
    if response.status_code() == 101 {
        return Ok(response);
    }
    let headers = Headers::new();
    for (name, value) in response.headers().entries() {
        headers.set(&name, &value)?;
    }
    headers.set("Access-Control-Allow-Origin", origin)?;
    headers.append("Vary", "Origin")?;
    Ok(response.with_headers(headers))
}

fn new_room_id() -> Result<String> {
    let mut buf = [0u8; 32];
    let mut used = buf.len();
    let mut failed = None;
    let id = random_room_id(|| {
        if used == buf.len() {
            if let Err(e) = getrandom::fill(&mut buf) {
                failed = Some(e);
            }
            used = 0;
        }
        used += 1;
        buf[used - 1]
    });
    match failed {
        Some(e) => Err(Error::RustError(format!("no randomness: {e}"))),
        None => Ok(id),
    }
}
