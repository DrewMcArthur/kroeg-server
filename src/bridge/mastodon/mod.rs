mod account;
mod statuses;
mod structs;
mod translate;
mod timelines;
mod oauth;

use futures::future;
use futures::prelude::{await, *};

use hyper::{Body, Request, Response, Uri};
use kroeg_tap::{Context, EntityStore, StoreItem};

#[async]
pub fn route<T: EntityStore>(
    context: Context,
    request: Request<Body>,
    store: T,
) -> Result<(Response<Body>, T), T::Error> {
    if request.uri().path().starts_with("/oauth") {
        await!(oauth::route(context, request, store))
    } else if request.uri().path().starts_with("/api/v1/account/") {
        await!(account::route(context, request, store))
    } else if request.uri().path().starts_with("/api/v1/statuses/") {
        await!(statuses::route(context, request, store))
    } else if request.uri().path().starts_with("/api/v1/timelines/") {
        await!(timelines::route(context, request, store))
    } else if request.uri().path() == "/api/v1/apps" {
        Ok((
            Response::builder()
                .status(200)
                .body(Body::from(
                    json!({"id": 1, "client_id": "no", "client_secret": "yes"}).to_string(),
                )).unwrap(),
            store,
        ))
    } else {
        Ok((
            Response::builder()
                .status(404)
                .body(Body::from("no idea"))
                .unwrap(),
            store,
        ))
    }
}
