use futures::future;
use futures::prelude::{await, *};

use hyper::{Body, Request, Response, Uri};
use kroeg_tap::{Context, EntityStore, StoreItem};

use super::translate;
use serde_json;

enum RequestType {
    Normal,
    Context,
    Card,
    RebloggedBy,
    FavouritedBy,
    Reblog,
    Unreblog,
    Favourite,
    Unfavourite,
    Pin,
    Unpin,
    Mute,
    Unmute
}

impl RequestType {
    fn parse(typ: &str) -> Option<RequestType> {
        match typ {
            "mute" => Some(RequestType::Mute),
            "unmute" => Some(RequestType::Unmute),
            "context" => Some(RequestType::Context),
            "card" => Some(RequestType::Card),
            "reblogged_by" => Some(RequestType::RebloggedBy),
            "favourited_by" => Some(RequestType::FavouritedBy),
            "reblog" => Some(RequestType::Reblog),
            "unreblog" => Some(RequestType::Unreblog),
            "favourite" => Some(RequestType::Favourite),
            "unfavourite" => Some(RequestType::Unfavourite),
            "pin" => Some(RequestType::Pin),
            "unpin" => Some(RequestType::Unpin),
            _ => None
        }
    }
}

#[async(boxed_send)]
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

    let (store, value) = await!(translate::status(store, path))?;
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
