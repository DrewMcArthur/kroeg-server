use context;
use context::HyperContextLoader;
use futures::future::{self, Either};
use futures::prelude::{await, *};
use futures::{stream, Future, Stream};
use hyper::{Body, Request, Response, Uri};
use jsonld::nodemap::Pointer;
use jsonld::{expand, JsonLdOptions};
use kroeg_tap::{
    assign_ids, untangle, Context, EntityStore, MessageHandler, QuadQuery, QueryId, QueryObject,
    QueueStore,
};
use kroeg_tap_activitypub::handlers;
use request::StoreAllFuture;
use serde_json;
use std::collections::{HashMap, HashSet};
use ServerError;

#[async]
fn prepare_delivery<T: EntityStore, Q: QueueStore>(
    context: Context,
    id: String,
    store: T,
    mut queue: Q,
    local: bool,
) -> Result<(Context, String, T, Q), (T::Error, T)> {
    let (obj, store) = await!(store.get(id.to_owned(), false))?;
    let obj = match obj {
        Some(val) => val,
        None => return Ok((context, id, store, queue)),
    };

    let mut boxes = HashSet::new();
    let mut audience: Vec<(usize, String, bool)> = Vec::new();

    // To process delivery, we use a queue-like structure. We resolve up to a depth of three, as to not allow very deep resolving.
    // (depth, id, should we try to resolve shared inboxes)

    for vals in &[
        as2!(to),
        as2!(bto),
        as2!(cc),
        as2!(bcc),
        as2!(audience),
        as2!(actor),
    ] {
        for item in &obj.main()[vals] {
            if let Pointer::Id(id) = item {
                audience.push((0, id.to_owned(), false));

                // Why not resolve shared inboxes at the first layer? it's not specced that way. boring, i know.
            }
        }
    }

    let (user_follower, mut store) = match await!(store.get(context.user.subject.to_owned(), true))?
    {
        (Some(user), store) => {
            if let Some(Pointer::Id(id)) = user.main()[as2!(followers)].iter().next() {
                (Some(id.to_owned()), store)
            } else {
                (None, store)
            }
        }
        (None, store) => (None, store),
    };

    while audience.len() > 0 {
        let (depth, item, is_shared) = audience.remove(0);
        let (item, _store) = await!(store.get(item, false))?;
        store = _store;

        if let Some(item) = item {
            // If this is a remote object, we will try to resolve shared inbox if available and allowed.
            // Local objects it makes no sense to do sharedInbox, as we will end up here again.
            // We will resolve local collections though!
            if !item.is_owned(&context) {
                if local {
                    let (query, _store) = await!(store.query(vec![QuadQuery(
                        QueryId::Placeholder(0),
                        QueryId::Value(String::from(as2!(followers))),
                        QueryObject::Id(QueryId::Value(item.id().to_owned())),
                    )]))?;

                    store = _store;

                    for mut item in query {
                        let user_id = match item.pop() {
                            Some(user_id) => user_id,
                            None => continue,
                        };

                        let (result, _store) = await!(store.read_collection_inverse(user_id))?;
                        store = _store;

                        let (query, _store) = await!(store.query(vec![
                            QuadQuery(
                                QueryId::Placeholder(0),
                                QueryId::Value(String::from(as2!(following))),
                                QueryObject::Id(QueryId::Any(result.items)),
                            ),
                            QuadQuery(
                                QueryId::Placeholder(0),
                                QueryId::Value(String::from(ldp!(inbox))),
                                QueryObject::Id(QueryId::Placeholder(1))
                            )
                        ]))?;
                        store = _store;

                        for mut item in query {
                            let _user = item.remove(0);
                            let inbox = item.remove(0);

                            boxes.insert(inbox);
                        }
                    }
                } else {
                    let mut has_shared = false;
                    if is_shared {
                        for endpoint in item.main()[as2!(endpoints)].clone() {
                            if let Pointer::Id(endpoint) = endpoint {
                                let (item, _store) = await!(store.get(endpoint, true))?;
                                store = _store;
                                let elem = item.unwrap();

                                for inbox in &elem.main()[as2!(sharedInbox)] {
                                    if let Pointer::Id(inbox) = inbox {
                                        boxes.insert(inbox.clone());
                                        has_shared = true;
                                    }
                                }
                            }
                        }
                    }

                    if has_shared {
                        continue;
                    }

                    for inbox in &item.main()[ldp!(inbox)] {
                        if let Pointer::Id(inbox) = inbox {
                            boxes.insert(inbox.to_owned());
                        }
                    }
                }
            } else {
                if !local {
                    if item
                        .main()
                        .types
                        .contains(&String::from(as2!(OrderedCollection)))
                        && depth < 3
                    {
                        let (data, _store) = await!(store.read_collection(
                            item.id().to_owned(),
                            Some(99999999),
                            None
                        ))?;
                        store = _store;

                        for fitem in data.items {
                            audience.push((
                                depth + 1,
                                fitem,
                                user_follower
                                    .as_ref()
                                    .map(|f| f == item.id())
                                    .unwrap_or(false),
                            ));
                        }
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

    for send_to in boxes.into_iter() {
        queue =
            await!(queue.add("deliver".to_owned(), format!("{} {}", obj.id(), send_to))).unwrap();
    }

    Ok((context, obj.id().to_owned(), store, queue))
}

enum DeliveryMode {
    LocalAndRemote,
    LocalOnly,
    None,
}

enum TrustMode {
    TrustIDs,
    AssignIDs,
}

fn get_handler<T: EntityStore>(
    box_type: &str,
) -> Option<(Vec<Box<MessageHandler<T>>>, DeliveryMode, TrustMode)> {
    match box_type {
        ldp!(inbox) => Some((
            vec![
                Box::new(handlers::VerifyRequiredEventsHandler(false)),
                Box::new(handlers::ServerCreateHandler),
            ],
            DeliveryMode::None,
            TrustMode::TrustIDs,
        )),

        as2!(outbox) => Some((
            vec![
                Box::new(handlers::AutomaticCreateHandler),
                Box::new(handlers::VerifyRequiredEventsHandler(true)),
                Box::new(handlers::ClientCreateHandler),
                Box::new(handlers::CreateActorHandler),
                Box::new(handlers::ClientLikeHandler),
                Box::new(handlers::ClientUndoHandler),
            ],
            DeliveryMode::LocalAndRemote,
            TrustMode::AssignIDs,
        )),

        as2!(sharedInbox) => Some((vec![], DeliveryMode::LocalOnly, TrustMode::TrustIDs)),

        _ => None,
    }
}

pub fn post<T: EntityStore, Q: QueueStore>(
    context: Context,
    store: T,
    queue: Q,
    request: Request<Body>,
) -> impl Future<Item = (T, Q, Response<serde_json::Value>), Error = (ServerError<T>, T)> {
    let id = format!("{}{}", context.server_base, request.uri().path());

    let (_, body) = request.into_parts();
    let expanded = body
        .concat2()
        .then(move |f| match f {
            Ok(body) => future::ok((body, store)),
            Err(e) => future::err((ServerError::HyperError(e), store)),
        })
        .and_then(|(val, store)| {
            match serde_json::from_slice(val.as_ref()).map(context::apply_supplement) {
                Ok(value) => Either::A(
                    expand::<HyperContextLoader>(
                        context::apply_supplement(value),
                        JsonLdOptions {
                            base: None,
                            compact_arrays: None,
                            expand_context: None,
                            processing_mode: None,
                        },
                    )
                    .then(move |f| match f {
                        Ok(ok) => future::ok((ok, store)),
                        Err(e) => future::err((ServerError::ExpansionError(e), store)),
                    }),
                ),
                Err(e) => Either::B(future::err((ServerError::SerdeError(e), store))),
            }
        });

    expanded
        .and_then(|(val, store)| {
            store
                .get(id, true)
                .map(|(item, store)| (store, val, item))
                .map_err(|(e, store)| (ServerError::StoreError(e), store))
        }).and_then(move |(store, val, item)| {
            if let Some(mut item) = item {
                let user_inoutbox = item.id().to_owned();
                let box_type = item.meta()[kroeg!(box)].get(0).and_then(|f| match f { Pointer::Id(id) => Some(id), _ => None });
                let handlers = box_type.and_then(|f| get_handler(f));
                let root = val
                    .as_array()
                    .and_then(|f| f.get(0))
                    .and_then(|f| f.as_object())
                    .and_then(|f| f.get("@id"))
                    .and_then(|f| f.as_str())
                    .map(|f| f.to_owned());

                let (handlers, delivery_mode, trust_mode) = match handlers {
                    Some(val) => val,
                    None => return Either::B(future::ok((
                        None,
                        context,
                        store,
                        vec![],
                        HashMap::new(),
                        user_inoutbox,
                    ))),
                };

                let mut untangled = untangle(val).unwrap();
                if let TrustMode::TrustIDs = trust_mode {
                    // Inboxes, aka server-to-server, we do not trust anything
                    // that isn't on the same origin as the authorized user.

                    let mut root = root.unwrap();
                    let user: Uri = context.user.subject.parse().unwrap();
                    let authority = user.authority_part().cloned();
                    untangled.retain(|k, _| {
                        if k.starts_with("_:") { &k[2..] } else { k }.parse::<Uri>()
                            .ok()
                            .and_then(|f| f.authority_part().cloned())
                            == authority
                    });
                    Either::B(future::ok((
                        Some((handlers, delivery_mode)),
                        context,
                        store,
                        vec![root],
                        untangled,
                        user_inoutbox,
                    )))
                } else {
                    // Outboxes, on the other hand, we do not trust any incoming
                    // IDs, and generate our own.
                    let user = context.user.subject.to_owned();
                    Either::A(
                        assign_ids(context, store, Some(user), untangled)
                            .map(move |(context, store, roots, untangled)|
                                (Some((handlers, delivery_mode)), context, store, roots, untangled, user_inoutbox)
                            ).map_err(|(e, store)| (ServerError::StoreError(e), store)),
                    )
                }
            } else {
                Either::B(future::ok((
                    None,
                    context,
                    store,
                    vec![],
                    HashMap::new(),
                    String::new(),
                )))
            }
        }).and_then(
            |(box_handler, context, store, roots, untangled, user_inoutbox)| {
                StoreAllFuture::new(store, untangled.into_iter().map(|(_, v)| v).collect())
                    .map(move |store| (box_handler, context, store, roots, user_inoutbox))
                    .map_err(|(e, store)| (ServerError::StoreError(e), store))
            },
        ).and_then(move |(box_handler, context, store, roots, user_inoutbox)| {
            let id = roots.into_iter().next();
            if id.is_none() {
                println!(" [ ] ignored POST cause we have no data?");
                return Either::A(Either::A(future::ok((context, store, queue, None, user_inoutbox))));
            }

            let (handlers, delivery_mode) = match box_handler {
                Some(val) => val,
                None => { return Either::A(Either::A(future::ok((context, store, queue, None, user_inoutbox)))); },
            };

            let id = id.unwrap();

            let end = stream::iter_ok(handlers.into_iter()).fold(
                ((context, store, id), user_inoutbox),
                |((context, store, id), user_inoutbox), handler| {
                    handler
                        .handle(context, store, user_inoutbox.to_owned(), id)
                        .map(|data| (data, user_inoutbox))
                        .map_err(|(e, store)| (ServerError::HandlerError(e), store))
                },
            );

            match delivery_mode {
                DeliveryMode::None => Either::B(Either::A(end.map(move |((context, store, id), user_inoutbox)| (context, store, queue, Some(id), user_inoutbox)))),
                DeliveryMode::LocalAndRemote => Either::A(Either::B(
                    end.and_then(|((context, store, id), user_inoutbox)|
                        prepare_delivery(context, id, store, queue, false)
                        .map_err(|(e, store)| (ServerError::StoreError(e), store))
                        .map(move |(context, id, store, queue)| (context, store, queue, Some(id), user_inoutbox))
                    )
                )),
                DeliveryMode::LocalOnly => Either::B(Either::B(
                    end.and_then(|((context, store, id), user_inoutbox)|
                        prepare_delivery(context, id, store, queue, true)
                        .map_err(|(e, store)| (ServerError::StoreError(e), store))
                        .map(move |(context, id, store, queue)| (context, store, queue, Some(id), user_inoutbox))
                    )
                )),
            }
        }).and_then(|(_, store, queue, id, user_inoutbox)| {
            if let Some(id) = id {
                Either::A(
                    store.insert_collection(user_inoutbox, id.to_owned())
                        .map(move |store| (store, queue, Response::builder()
                            .status(201)
                            .header("Location", &id as &str)
                            .header("Content-Type", "application/ld+json; profile=\"https://www.w3.org/ns/activitystreams\"")
                            .body(json!({ "@id": id }))
                            .unwrap()))
                        .map_err(|(e, store)| (ServerError::StoreError(e), store))
                )
            } else {
                Either::B(future::ok((store, queue,
                    Response::builder()
                        .status(404)
                        .body(json!({ "@type": [kroeg!(NotFound)] }))
                        .unwrap()
                )))
            }
        })
}
