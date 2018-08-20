use futures::future;
use futures::prelude::{await, *};

use hyper::{Body, Request, Response, Uri};
use kroeg_tap::{Context, EntityStore, StoreItem};

use super::translate;
use serde_json;


#[async]
pub fn route<T: EntityStore>(
    context: Context,
    request: Request<Body>,
    store: T,
) -> Result<(Response<Body>, T), T::Error> {
    Ok((Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string(&json!({
            "access_token": "[todo]",
            "token_type": "bearer",
            "expires_in": 3600 * 3600,
        })).unwrap()))
        .unwrap(),
        store,
    ))
}
