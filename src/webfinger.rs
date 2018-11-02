use futures::prelude::*;

use super::{router::Route, KroegServiceBuilder, ServerError};
use futures::future::{self, Either};
use hyper::{Body, Request, Response, Uri};
use jsonld::nodemap::{Pointer, Value};
use kroeg_tap::{Context, EntityStore, QuadQuery, QueryId, QueryObject, QueueStore, StoreItem};
use serde_json::Value as JValue;

/// Extracts a query in the shape of e.g. resource=acct:a@b&other=aaaa into ("a", "b")
fn extract_acct<'a>(query: &'a str) -> Option<(&'a str, &'a str)> {
    for spl in query.split('&') {
        if spl.starts_with("resource=acct:") {
            let vals: Vec<_> = spl[14..].split('@').collect();
            if vals.len() == 2 {
                return Some((vals[0], vals[1]));
            }
        }
    }

    None
}

fn extract_username(user: &StoreItem) -> Option<String> {
    match user.main()[as2!(preferredUsername)].iter().next() {
        Some(Pointer::Value(Value {
            value: JValue::String(username),
            ..
        })) => Some(username.to_owned()),
        _ => None,
    }
}

fn handle_webfinger<T: EntityStore, R: QueueStore>(
    context: Context,
    store: T,
    queue: R,
    request: Request<Body>,
) -> Box<Future<Item = (T, R, Response<Body>), Error = (ServerError<T>, T)> + Send + 'static> {
    let acct = extract_acct(request.uri().query().unwrap_or(""));

    Box::new(
        match acct {
            Some((user, _)) => Either::A(store.query(vec![
                QuadQuery(
                    QueryId::Placeholder(0),
                    QueryId::Value(String::from(as2!(preferredUsername))),
                    QueryObject::Object {
                        value: user.to_owned(),
                        type_id: QueryId::Value(
                            "http://www.w3.org/2001/XMLSchema#string".to_owned(),
                        ),
                    },
                ),
                QuadQuery(
                    QueryId::Placeholder(0),
                    QueryId::Value(String::from(kroeg!(instance))),
                    QueryObject::Object {
                        value: context.instance_id.to_string(),
                        type_id: QueryId::Value(
                            "http://www.w3.org/2001/XMLSchema#integer".to_owned(),
                        ),
                    },
                ),
            ])),
            None => Either::B(future::ok((vec![], store))),
        }
        .and_then(move |(item, store)| {
            let item = item.into_iter().next().and_then(|f| f.into_iter().next());
            if let Some(item) = item {
                Either::A(store.get(item, true))
            } else {
                Either::B(future::ok((None, store)))
            }
        })
        .map(move |(item, store)| {
            let item = item.and_then(|f| extract_username(&f).map(|val| (f, val)));
            let response = if let Some((user, username)) = item {
                let uri: Uri = user.id().parse().unwrap();

                let response = json!({
                "subject": format!("acct:{}@{}", username, uri.authority_part().unwrap()),
                "aliases": [user.id()],
                "links": [{
                    "rel": "self",
                    "type": "application/activity+json",
                    "href": user.id()
                }]
            });

                Response::builder()
                    .status(200)
                    .header("Content-Type", "application/jrd+json")
                    .body(Body::from(response.to_string()))
                    .unwrap()
            } else {
                Response::builder()
                    .status(404)
                    .body(Body::from("not found"))
                    .unwrap()
            };

            (store, queue, response)
        })
        .map_err(|(e, store)| (ServerError::StoreError(e), store)),
    )
}

pub fn register(builder: &mut KroegServiceBuilder) {
    builder.routes.push(Route::get(
        "/.well-known/webfinger",
        Box::new(handle_webfinger),
    ));
}
