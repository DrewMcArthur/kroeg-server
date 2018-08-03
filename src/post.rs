//! Code to handle GET requests for a server.

use futures::future;
use futures::prelude::*;

use hyper::{Body, Request, Response, Uri};
use kroeg_tap::{Context, EntityStore, StoreItem};
use serde_json::{from_slice, Value};

use super::context::HyperContextLoader;
use jsonld::nodemap::Pointer;
use jsonld::{expand, JsonLdOptions};

use kroeg_tap::{assemble, assign_ids, untangle, DefaultAuthorizer, MessageHandler, QueueStore};
use kroeg_tap_activitypub::handlers;

use super::delivery::register_delivery;

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

#[async]
fn audience_for_object<T: EntityStore>(
    context: Context,
    obj: StoreItem,
    store: T,
) -> Result<(Context, StoreItem, T, HashSet<String>), T::Error> {
    let mut boxes = HashSet::new();
    let mut audience = Vec::new();
    for vals in &[as2!(to), as2!(bto), as2!(cc), as2!(bcc), as2!(audience)] {
        for item in &obj.main()[vals] {
            if let Pointer::Id(id) = item {
                audience.push((0, id.to_owned()));
            }
        }
    }

    while audience.len() > 0 {
        let (depth, item) = audience.remove(0);
        let item = await!(store.get(item))?;
        if let Some(item) = item {
            if !item.is_owned(&context) {
                for inbox in &item.main()[ldp!(inbox)] {
                    if let Pointer::Id(inbox) = inbox {
                        boxes.insert(inbox.to_owned());
                    }
                }
            } else {
                if item
                    .main()
                    .types
                    .contains(&String::from(as2!(OrderedCollection)))
                {
                    let data =
                        await!(store.read_collection(item.id().to_owned(), Some(99999999), None))?;
                    for item in data.items {
                        audience.push((0, item));
                    }
                }

                for inbox in &item.main()[ldp!(inbox)] {
                    if let Pointer::Id(inbox) = inbox {
                        boxes.insert(inbox.to_owned());
                    }
                }
            }
        }
    }

    Ok((context, obj, store, boxes))
}

#[async(boxed_send)]
fn run_handlers<T: EntityStore, R: QueueStore>(
    context: Context,
    store: T,
    mut queue: R,
    inbox: String,
    expanded: Value,
) -> Result<(T, R, Response<Value>), ServerError<T>> {
    let user = Some(context.user.subject.to_owned());
    let mut source = await!(store.get(inbox.to_owned()))
        .map_err(ServerError::StoreError)?
        .unwrap();
    let mut is_outbox = false;
    let (mut context, mut store, id) = if source.meta()[kroeg!(box)]
        .contains(&Pointer::Id(as2!(outbox).to_owned()))
    {
        is_outbox = true;
        let mut untangled = untangle(expanded).unwrap();
        let (context, store, mut roots, data) =
            await!(assign_ids(context, store, user, untangled)).map_err(ServerError::StoreError)?;
        let store = await!(store_all(store, data)).map_err(ServerError::StoreError)?;
        let id = roots.remove(0);

        run_handlers! {
            context, store, inbox.to_owned(), id,
            handlers::AutomaticCreateHandler,
            handlers::VerifyRequiredEventsHandler,
            handlers::ClientCreateHandler,
            handlers::CreateActorHandler,
            handlers::ClientLikeHandler,
            handlers::ClientUndoHandler
        }
    } else if source.meta()[kroeg!(box)].contains(&Pointer::Id(ldp!(inbox).to_owned())) {
        let host = {
            let host = context.user.subject.parse::<Uri>().unwrap();
            host.authority_part().unwrap().clone()
        };
        println!("{}", expanded.to_string());
        let root = expanded.as_array().unwrap()[0].as_object().unwrap()["@id"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut untangled = untangle(expanded).unwrap();
        untangled.retain(|k, v| k.parse::<Uri>().unwrap().authority_part().unwrap() == &host);
        if !untangled.contains_key(&root) {
            return Err(ServerError::BadSharedInbox);
        }
        let store = await!(store_all(store, untangled)).map_err(ServerError::StoreError)?;
        let id = root;
        run_handlers! {
            context, store, inbox.to_owned(), id,
            handlers::VerifyRequiredEventsHandler,
            handlers::ServerCreateHandler
        }
    } else {
        return Err(ServerError::PostToNonbox);
    };

    await!(store.insert_collection(inbox.to_owned(), id.to_owned()))
        .map_err(ServerError::StoreError)?;

    let mut item = await!(store.get(id.to_owned()))
        .map_err(ServerError::StoreError)?
        .unwrap();

    if is_outbox {
        println!(" ┃ Preparing delivery..");
        let (c, i, s, boxes) =
            await!(audience_for_object(context, item, store)).map_err(ServerError::StoreError)?;
        context = c;
        item = i;
        store = s;

        for inbox in boxes {
            println!(" ┃ Delivering to {}", inbox);
            queue = await!(register_delivery(queue, item.id().to_owned(), inbox)).unwrap();
        }
    }

    Ok((
        store,
        queue,
        Response::builder()
            .status(201)
            .header("Location", &id as &str)
            .header(
                "Content-Type",
                "application/ld+json; profile=\"https://www.w3.org/ns/activitystreams\"",
            )
            .body(json!({ "@id": id }))
            .unwrap(),
    ))
}

#[async]
fn store_all<T: EntityStore>(
    mut store: T,
    items: HashMap<String, StoreItem>,
) -> Result<T, T::Error> {
    for (key, mut value) in items {
        let item = await!(store.get(key.to_owned()))?;
        if let Some(mut item) = item {
            if item.meta()[kroeg!(instance)] != value.meta()[kroeg!(instance)] {
                println!("not storing {} because self-bug", item.id());
                continue;
            }
        }
        await!(store.put(key, value))?;
    }

    Ok(store)
}

pub fn process<T: EntityStore, R: QueueStore>(
    context: Context,
    store: T,
    queue: R,
    req: Request<Body>,
) -> impl Future<Item = (T, R, Response<Value>), Error = ServerError<T>> {
    let uri = req.uri().to_owned();
    let name = format!("{}{}", context.server_base, uri.path());

    println!(" ┗ POST {}", name);

    let body = req.into_body();
    body.concat2()
        .map_err(|e| ServerError::HyperError(e))
        .and_then(
            |val| -> Box<Future<Item = Value, Error = ServerError<T>> + Send> {
                let data: Value = match from_slice(val.as_ref()) {
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
        .and_then(|expanded| run_handlers(context, store, queue, name, expanded))
}
