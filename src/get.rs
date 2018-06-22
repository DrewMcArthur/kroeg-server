//! Code to handle GET requests for a server.

use futures::prelude::*;

use hyper::{Body, Request, Response};
use kroeg_tap::{assemble, Context, EntityStore};
use serde_json::Value;
use std::collections::HashSet;

use super::ServerError;

#[async(boxed_send)]
pub fn process<T: EntityStore>(
    context: Context,
    mut store: T,
    req: Request<Body>,
) -> Result<(T, Response<Value>), ServerError<T>> {
    let uri = req.uri().to_owned();
    let name = format!("{}{}", context.server_base, uri.path());

    let val = await!(store.get(name.to_owned())).map_err(|e| ServerError::StoreError(e))?;
    let val = match val {
        Some(data) => {
            let (_, nstore, data) = await!(assemble(data, 0, Some(store), HashSet::new()))
                .map_err(|e| ServerError::StoreError(e))?;
            store = nstore.unwrap();

            Some(data)
        }

        None => None,
    };

    let mut builder = Response::builder();

    builder.header("Content-Type", "application/ld+json; profile=\"https://www.w3.org/ns/activitystreams\"");

    let response = match val {
        Some(data) => builder.status(200).body(data),
        None => builder.status(404).body(json!({
                    "@type": "https://puckipedia.com/kroeg/ns#NotFound", 
                    as2!(content): "Not found"
                })),
    }.unwrap();

    Ok((store, response))
}
