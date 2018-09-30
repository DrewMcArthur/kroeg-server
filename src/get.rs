//! Code to handle GET requests for a server.

use futures::future;
use futures::future::Either;
use futures::prelude::*;

use hyper::{Body, Request, Response};
use jsonld::nodemap::Pointer;
use kroeg_tap::{
    assemble, Authorizer, Context, DefaultAuthorizer, EntityStore, QueueStore, StoreItem,
};
use serde_json::Value;
use std::collections::HashSet;

use super::ServerError;

fn build_collection_page<T: EntityStore>(
    store: T,
    item: StoreItem,
    query: String,
) -> impl Future<Item = (T, Option<StoreItem>), Error = (T::Error, T)> + Send {
    let cursor = if query == "first" {
        None
    } else {
        Some(query.to_owned())
    };

    store
        .read_collection(item.id().to_owned(), None, cursor)
        .map(move |(page, store)| {
            let full_id = format!("{}?{}", item.id(), query);
            let items: Vec<_> = page
                .items
                .into_iter()
                .map(|f| json!({ "@id": f }))
                .collect();

            let json = json!({
                "@id": full_id,
                "@type": [as2!(OrderedCollectionPage)],
                as2!(partOf): [{ "@id": item.id() }],
                as2!(items): [{ "@list": items }]
            });

            let mut page_item = StoreItem::parse(&full_id, json).unwrap();

            if let Some(prev) = page.before {
                page_item.main_mut()[as2!(prev)].push(Pointer::Id(format!(
                    "{}?{}",
                    item.id(),
                    prev
                )));
            }

            if let Some(next) = page.after {
                page_item.main_mut()[as2!(next)].push(Pointer::Id(format!(
                    "{}?{}",
                    item.id(),
                    next
                )));
            }

            (store, Some(page_item))
        })
}

fn ensure_authorized<T: EntityStore>(
    context: Context,
    store: T,
    item: Option<StoreItem>,
) -> impl Future<Item = (Context, T, Option<StoreItem>), Error = (T::Error, T)> + Send {
    match item {
        Some(value) => {
            let authorizer = DefaultAuthorizer::new(&context);
            Either::A(authorizer.can_show(store, &value).map(|(store, can_show)| {
                (context, store, if can_show { Some(value) } else { None })
            }))
        }

        None => Either::B(future::ok((context, store, None))),
    }
}

pub fn get<T: EntityStore, R: QueueStore>(
    context: Context,
    store: T,
    queue: R,
    request: Request<Body>,
) -> impl Future<Item = (T, R, Response<Value>), Error = (ServerError<T>, T)> + Send {
    let id = format!("{}{}", context.server_base, request.uri().path());
    let query = request.uri().query().map(str::to_string);

    store
        .get(id, true)
        .and_then(move |(item, store)| match (item, query) {
            (Some(item), Some(query)) => {
                Either::A(build_collection_page(store, item, query.to_string()))
            }
            (item, _) => Either::B(future::ok((store, item))),
        }).and_then(move |(store, value)| ensure_authorized(context, store, value))
        .and_then(move |(context, store, value)| {
            let mut response = Response::builder();
            response
                .header("Vary", "Accept")
                .header("Content-Type", "application/activity+json");

            match value {
                Some(mut value) => {
                    if value.is_owned(&context) && value
                        .main()
                        .types
                        .contains(&String::from(as2!(OrderedCollection)))
                    {
                        let id = format!("{}?first", value.id());
                        value.main_mut()[as2!(first)].push(Pointer::Id(id));
                    }

                    Either::A(
                        assemble(
                            value,
                            0,
                            Some(store),
                            DefaultAuthorizer::new(&context),
                            HashSet::new(),
                        ).map(move |(_, store, _, data)| {
                            let response = response.status(200).body(data).unwrap();

                            (store.unwrap(), queue, response)
                        }),
                    )
                }

                None => {
                    let response = response
                        .status(404)
                        .body(json!({
                            "@type": "https://puckipedia.com/kroeg/ns#NotFound", 
                            as2!(content): "Not found"
                        })).unwrap();

                    Either::B(future::ok((store, queue, response)))
                }
            }
        }).map_err(|(e, store)| (ServerError::StoreError(e), store))
}
