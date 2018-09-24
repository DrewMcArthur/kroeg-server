use context;
use context::HyperContextLoader;
use futures::future::{self, Either};
use futures::{stream, Future, Stream};
use hyper::{Body, Request, Response, Uri};
use jsonld::nodemap::Pointer;
use jsonld::{expand, JsonLdOptions};
use kroeg_tap::{assign_ids, untangle, Context, EntityStore, MessageHandler, QueueStore};
use kroeg_tap_activitypub::handlers;
use request::StoreAllFuture;
use serde_json;
use std::collections::HashMap;
use ServerError;

pub fn post<T: EntityStore, Q: QueueStore>(
    context: Context,
    store: T,
    queue: Q,
    request: Request<Body>,
) -> impl Future<Item = (T, Q, Response<serde_json::Value>), Error = ServerError<T>> {
    let id = format!("{}{}", context.server_base, request.uri().path());

    let (_, body) = request.into_parts();
    let expanded =
        body.concat2().map_err(ServerError::HyperError).and_then(
            |val| match serde_json::from_slice(val.as_ref()).map(context::apply_supplement) {
                Ok(value) => Either::A(
                    expand::<HyperContextLoader>(
                        context::apply_supplement(value),
                        JsonLdOptions {
                            base: None,
                            compact_arrays: None,
                            expand_context: None,
                            processing_mode: None,
                        },
                    ).map_err(ServerError::ExpansionError),
                ),
                Err(e) => Either::B(future::err(ServerError::SerdeError(e))),
            },
        );

    expanded
        .and_then(move |val| {
            store
                .get(id, true)
                .map(|item| (store, val, item))
                .map_err(ServerError::StoreError)
        }).and_then(move |(store, val, item)| {
            if let Some(mut item) = item {
                let user_inoutbox = item.id().to_owned();
                let is_box = &item.meta()[kroeg!(box)];
                let is_inbox = is_box.contains(&Pointer::Id(as2!(inbox).to_owned()));
                let is_outbox = is_box.contains(&Pointer::Id(as2!(outbox).to_owned()));
                let root = val
                    .as_array()
                    .and_then(|f| f.get(0))
                    .and_then(|f| f.as_object())
                    .and_then(|f| f.get("@id"))
                    .and_then(|f| f.as_str())
                    .map(|f| f.to_owned());

                if (!is_inbox && !is_outbox) || (is_inbox && is_outbox) {
                    return Either::B(future::ok((
                        false,
                        context,
                        store,
                        vec![],
                        HashMap::new(),
                        user_inoutbox,
                    )));
                }

                let mut untangled = untangle(val).unwrap();
                if is_inbox {
                    // Inboxes, aka server-to-server, we do not trust anything
                    // that isn't on the same origin as the authorized user.

                    let mut root = root.unwrap();
                    let user: Uri = context.user.subject.parse().unwrap();
                    let authority = user.authority_part().cloned();
                    untangled.retain(|k, _| {
                        k.parse::<Uri>()
                            .ok()
                            .and_then(|f| f.authority_part().cloned())
                            == authority
                    });
                    Either::B(future::ok((
                        is_inbox,
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
                            .map(move |(context, store, roots, untangled)| {
                                (is_inbox, context, store, roots, untangled, user_inoutbox)
                            }).map_err(ServerError::StoreError),
                    )
                }
            } else {
                Either::B(future::ok((
                    false,
                    context,
                    store,
                    vec![],
                    HashMap::new(),
                    String::new(),
                )))
            }
        }).and_then(
            |(is_inbox, context, store, roots, untangled, user_inoutbox)| {
                StoreAllFuture::new(store, untangled.into_iter().map(|(_, v)| v).collect())
                    .map(move |store| (is_inbox, context, store, roots, user_inoutbox))
                    .map_err(ServerError::StoreError)
            },
        ).and_then(move |(is_inbox, context, store, roots, user_inoutbox)| {
            let handlers: Vec<Box<MessageHandler<T>>> = if is_inbox {
                vec![
                    Box::new(handlers::VerifyRequiredEventsHandler(false)),
                    Box::new(handlers::ServerCreateHandler),
                    Box::new(handlers::ServerFollowHandler),
                ]
            } else {
                vec![
                    Box::new(handlers::AutomaticCreateHandler),
                    Box::new(handlers::VerifyRequiredEventsHandler(true)),
                    Box::new(handlers::ClientCreateHandler),
                    Box::new(handlers::CreateActorHandler),
                    Box::new(handlers::ClientLikeHandler),
                    Box::new(handlers::ClientUndoHandler),
                ]
            };

            let id = roots.into_iter().next();
            if id.is_none() {
                return Either::A(future::ok((context, store, None, user_inoutbox)));
            }

            let id = id.unwrap();

            let end = stream::iter_ok(handlers.into_iter()).fold(
                ((context, store, id), user_inoutbox),
                |((context, store, id), user_inoutbox), handler| {
                    handler
                        .handle(context, store, user_inoutbox.to_owned(), id)
                        .map(|data| (data, user_inoutbox))
                        .map_err(|e| ServerError::HandlerError(e))
                },
            );

            Either::B(end.map(|((context, store, id), user_inoutbox)| (context, store, Some(id), user_inoutbox)))
        }).and_then(move |(_, mut store, id, user_inoutbox)| {
            if let Some(id) = id {
                Either::A(
                    store.insert_collection(user_inoutbox, id.to_owned())
                        .map(move |_| (store, queue, Response::builder()
                            .status(201)
                            .header("Location", &id as &str)
                            .header("Content-Type", "application/ld+json; profile=\"https://www.w3.org/ns/activitystreams\"")
                            .body(json!({ "@id": id }))
                            .unwrap()))
                        .map_err(ServerError::StoreError)
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
