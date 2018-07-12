//! Code to handle GET requests for a server.

use futures::future;
use futures::prelude::*;

use hyper::{Body, Request, Response};
use kroeg_tap::{Context, EntityStore, StoreItem};
use serde_json::{from_slice, Value};

use super::context::HyperContextLoader;
use jsonld::nodemap::Pointer;
use jsonld::{expand, JsonLdOptions};

use kroeg_tap::{assemble, assign_ids, untangle, MessageHandler};
use kroeg_tap_activitypub::handlers;

use std::collections::{HashMap, HashSet};

use super::ServerError;

macro_rules! run_handlers {
    ($context:expr, $store:expr, $inbox:expr, $id:expr, $($exp:expr),*) => {
        {
        let mut context = $context;
        let mut store = $store;
        let inbox = $inbox;
        let mut id = $id;
        $({
            let item = $exp;
            let (ncontext, nstore, nid) = await!(item.handle(context, store, inbox.to_owned(), id))
                .map_err(|e| ServerError::HandlerError(Box::new(e)))?;
            id = nid;
            context = ncontext;
            store = nstore;
        })*

        (context, store, id)
        }
    };
}

#[async(boxed_send)]
fn run_handlers<T: EntityStore>(
    context: Context,
    store: T,
    inbox: String,
    id: String,
) -> Result<(T, Response<Value>), ServerError<T>> {
    let source = await!(store.get(inbox.to_owned()))
        .map_err(ServerError::StoreError)?
        .unwrap();
    match source.sub(kroeg!(meta)) {
        Some(val) => {
            if !val[kroeg!(box)].contains(&Pointer::Id(as2!(outbox).to_owned())) {
                return Err(ServerError::PostToNonbox);
            }
        }

        None => return Err(ServerError::PostToNonbox),
    }

    let (_, mut store, id) = run_handlers! {
        context, store, inbox.to_owned(), id,
        handlers::AutomaticCreateHandler,
        handlers::VerifyRequiredEventsHandler,
        handlers::CreateActorHandler
    };

    await!(store.insert_collection(inbox.to_owned(), id.to_owned()))
        .map_err(ServerError::StoreError)?;

    let item = await!(store.get(id.to_owned()))
        .map_err(ServerError::StoreError)?
        .unwrap();
    let (_, store, val) =
        await!(assemble(item, 0, Some(store), HashSet::new())).map_err(ServerError::StoreError)?;

    Ok((store.unwrap(),
    Response::builder()
        .status(201)
        .header("Location", &id as &str)
        .header("Content-Type", "application/ld+json; profile=\"https://www.w3.org/ns/activitystreams\"")
        .body(val)
        .unwrap()))
}

#[async]
fn store_all<T: EntityStore>(
    mut store: T,
    items: HashMap<String, StoreItem>,
) -> Result<T, T::Error> {
    for (key, value) in items {
        await!(store.put(key, value))?;
    }

    Ok(store)
}

pub fn process<T: EntityStore>(
    context: Context,
    store: T,
    req: Request<Body>,
) -> impl Future<Item = (T, Response<Value>), Error = ServerError<T>> {
    let uri = req.uri().to_owned();
    let name = format!("{}{}", context.server_base, uri.path());

    println!("-- POST {}", name);

    let body = req.into_body();
    body.concat2()
        .map_err(|e| ServerError::HyperError(e))
        .and_then(
            |val| -> Box<Future<Item = Value, Error = ServerError<T>> + Send> {
                let data = match from_slice(val.as_ref()) {
                    Ok(data) => data,
                    Err(err) => return Box::new(future::err(ServerError::SerdeError(err))),
                };

                Box::new(
                    expand::<HyperContextLoader>(
                        data,
                        JsonLdOptions {
                            base: None,
                            compact_arrays: None,
                            expand_context: None,
                            processing_mode: None,
                        },
                    ).map_err(ServerError::ExpansionError),
                )
            },
        )
        .and_then(|expanded| {
            let untangled = untangle(expanded);
            let user = Some(context.user.subject.to_owned());
            assign_ids(context, store, user, untangled.unwrap()).map_err(ServerError::StoreError)
        })
        .and_then(|(context, store, roots, data)| {
            store_all(store, data)
                .map(|f| (context, f, roots))
                .map_err(ServerError::StoreError)
        })
        .and_then(|(context, store, mut roots)| run_handlers(context, store, name, roots.remove(0)))
}
