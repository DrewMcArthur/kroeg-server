use futures::future;
use futures::prelude::{await, *};

use hyper::{Body, Request, Response, Uri};
use kroeg_tap::{Context, EntityStore, StoreItem};

use super::translate;
use serde_json;

/*
use serde_json::{from_slice, Value};

use super::context::HyperContextLoader;
use jsonld::nodemap::Pointer;
use jsonld::{expand, JsonLdOptions};

use kroeg_tap::{assemble, assign_ids, untangle, DefaultAuthorizer, MessageHandler, QueueStore};
use kroeg_tap_activitypub::handlers;

use super::delivery::register_delivery;

use std::collections::{HashMap, HashSet};

use super::ServerError;*/

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

#[async]
pub fn route<T: EntityStore>(
    context: Context,
    request: Request<Body>,
    store: T,
) -> Result<(Response<Body>, T), T::Error> {
    let path = request.uri().path()[16..].to_string();
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
