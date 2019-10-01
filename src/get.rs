//! Code to handle GET requests for a server.

use http_service::{Body, Request, Response};
use jsonld::nodemap::Pointer;
use kroeg_tap::{as2, assemble, Authorizer, Context, DefaultAuthorizer, StoreItem};
use serde_json::json;
use std::collections::HashSet;

use crate::ServerError;

async fn build_collection_page(
    context: &mut Context<'_, '_>,
    item: StoreItem,
    query: String,
) -> Result<StoreItem, ServerError> {
    let cursor = if query == "first" {
        None
    } else {
        Some(query.to_owned())
    };

    let page = context
        .entity_store
        .read_collection(item.id().to_owned(), None, cursor)
        .await
        .map_err(ServerError::StoreError)?;

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

    let mut page_item = StoreItem::parse(&full_id, &json).unwrap();

    if let Some(prev) = page.before {
        page_item.main_mut()[as2!(prev)].push(Pointer::Id(format!("{}?{}", item.id(), prev)));
    }

    if let Some(next) = page.after {
        page_item.main_mut()[as2!(next)].push(Pointer::Id(format!("{}?{}", item.id(), next)));
    }

    Ok(page_item)
}

fn not_found() -> Response {
    http::Response::builder()
        .status(404)
        .body(Body::from(
            json!({
                "@type": "https://puckipedia.com/kroeg/ns#NotFound",
                as2!(content): "Not found"
            })
            .to_string(),
        ))
        .unwrap()
}

pub struct GetHandler;

#[async_trait::async_trait]
impl crate::router::RequestHandler for GetHandler {
    async fn run(
        &self,
        context: &mut Context<'_, '_>,
        request: Request,
    ) -> Result<Response, ServerError> {
        let mut id = format!("{}{}", context.server_base, request.uri().path());
        let mut query = request.uri().query().map(str::to_string);

        let mut item = match context
            .entity_store
            .get(id, false)
            .await
            .map_err(ServerError::StoreError)?
        {
            Some(item)
                if DefaultAuthorizer
                    .can_show(context, &item)
                    .await
                    .map_err(ServerError::StoreError)? =>
            {
                item
            }
            _ => return Ok(not_found()),
        };

        if item.is_owned(context)
            && item
                .main()
                .types
                .iter()
                .any(|f| f == as2!(OrderedCollection))
        {
            if let Some(query) = query {
                item = build_collection_page(context, item, query).await?;
            } else {
                let id = format!("{}?first", item.id());
                item.main_mut()[as2!(first)].push(Pointer::Id(id));
            }
        }

        let assembled = assemble(&item, 0, context, &DefaultAuthorizer, &mut HashSet::new())
            .await
            .map_err(ServerError::StoreError)?;

        let compacted = crate::context::compact(context, &assembled).await.unwrap();

        Ok(http::Response::builder()
            .header("Vary", "Accept")
            .header("Content-Type", "application/activity+json")
            .body(Body::from(compacted.to_string()))
            .unwrap())
    }
}
