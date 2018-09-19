use futures::future;
use futures::prelude::{await, *};

use hyper::{Body, Request, Response, Uri};
use kroeg_tap::{Context, EntityStore, StoreItem};

use super::translate;
use serde_json;

enum RequestType {
    Normal,
    Followers,
    Following,
    Statuses,
    Follow,
    Unfollow,
    Block,
    Unblock,
    Mute,
    Unmute,
}

impl RequestType {
    fn parse(typ: &str) -> Option<RequestType> {
        match typ {
            "followers" => Some(RequestType::Followers),
            "following" => Some(RequestType::Following),
            "statuses" => Some(RequestType::Statuses),
            "follow" => Some(RequestType::Follow),
            "unfollow" => Some(RequestType::Unfollow),
            "block" => Some(RequestType::Block),
            "unblock" => Some(RequestType::Unblock),
            "mute" => Some(RequestType::Mute),
            "unmute" => Some(RequestType::Unmute),
            _ => None
        }
    }
}

#[async]
pub fn route<T: EntityStore>(
    context: Context,
    request: Request<Body>,
    store: T,
) -> Result<(Response<Body>, T), T::Error> {
    println!("routed into account");
    let (path, message_type) = {
	let parts: Vec<_> = request.uri().path()[17..].split('/').collect();
	match parts.len() {
            1 => (translate::decode_id(parts[0].to_string()), RequestType::Normal),
            2 => if let Some(val) = RequestType::parse(&parts[1]) {
                (translate::decode_id(parts[0].to_string()), val)
            } else {
                return Ok((Response::builder().status(401).body(Body::from("bad request type")).unwrap(), store))
            }
            _ => return Ok((Response::builder().status(401).body(Body::from("bad request")).unwrap(), store))
	}
    };
    println!("{}", path);

    let (store, value) = await!(translate::account(store, path))?;
    Ok((
        match value {
            Some(val) => Response::builder()
                .status(200)
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&val).unwrap()))
                .unwrap(),

            None => Response::builder()
                .status(404)
                .body(Body::from("Account not found"))
                .unwrap(),
        },
        store,
    ))
}
