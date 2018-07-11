//! Code to handle GET requests for a server.

use futures::prelude::*;

use hyper::{Body, Request, Response};
use kroeg_tap::{assemble, Context, EntityStore, StoreItem};
use jsonld::nodemap::Pointer;
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
    let mut name = format!("{}{}", context.server_base, uri.path());
    let query = uri.query().map(|f| f.to_string());
    let val = if let Some(query) = query {
        let val = await!(store.get(name.to_owned())).map_err(ServerError::StoreError)?;
        if let Some(val) = val {
            if !val.main().types.contains(&as2!(OrderedCollection).to_string()) {
                None
            } else {
                let data = await!(store.read_collection(name.to_owned(), None, if query == "first" { None } else { Some(query.to_owned()) }))
                    .map_err(ServerError::StoreError)?;

                let withquery = format!("{}{}?{}", context.server_base, uri.path(), query);

                let items: Vec<_> = data.items.iter().map(|f| json!({"@id": f})).collect();
                let mut elem = StoreItem::parse(&withquery, json!({
                    "@id": withquery,
                    "@type": [as2!(OrderedCollectionPage)],
                    as2!(partOf): [{"@id": name}],
                    as2!(items): [{"@list": items}]
                })).expect("static input cannot fail");

                if let Some(cursor) = data.before {
                    elem.main_mut()[as2!(prev)].push(Pointer::Id(format!("{}?{}", name, cursor)));
                }

                if let Some(cursor) = data.after {
                    elem.main_mut()[as2!(next)].push(Pointer::Id(format!("{}?{}", name, cursor)));
                }

                Some(elem)
            }
        } else {
            None
        }
    } else {
        await!(store.get(name.to_owned())).map_err(|e| ServerError::StoreError(e))?
    };

    let val = match val {
        Some(data) => {
            let (_, nstore, data) = await!(assemble(data, 0, Some(store), HashSet::new()))
                .map_err(ServerError::StoreError)?;
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
