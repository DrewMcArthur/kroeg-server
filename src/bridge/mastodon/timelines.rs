use futures::future;
use futures::prelude::{await, *};

use hyper::{Body, Request, Response, Uri};
use kroeg_tap::{Context, EntityStore, StoreItem};

use super::translate;
use serde_json;

enum RequestType {
    Home,
    Public(bool),
    Tag(String),
    List(String),
}

#[async]
pub fn route<T: EntityStore>(
    context: Context,
    request: Request<Body>,
    store: T,
) -> Result<(Response<Body>, T), T::Error> {
    let request_type = if request.uri().path() == "/api/v1/timelines/home" {
        RequestType::Home
    } else if request.uri().path() == "/api/v1/timelines/public" {
        RequestType::Public(true)
    } else if request.uri().path().starts_with("/api/v1/timelines/tag/") {
        RequestType::Tag(request.uri().path()[22..].to_owned())
    } else if request.uri().path().starts_with("/api/v1/timelines/list/") {
        RequestType::List(request.uri().path()[23..].to_owned())
    } else {
        return Ok((Response::builder().status(404).body(Body::from("unknown route")).unwrap(), store));
    };

    let path = match request_type {
        // RequestType::Public(_) => "https://puckipedia.com/_dumpAll".to_owned(),
        _ => "https://puckipedia.com/inbox".to_owned()
    };

    let (store, val, _, _) = await!(translate::timeline(store, path, None, translate::status))?;
    Ok((
        Response::builder()
            .status(200)
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_string(&val).unwrap()))
            .unwrap(),
       store
    ))
}
