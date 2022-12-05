use http::Uri;
use http_service::{Body, Request, Response};
use jsonld::nodemap::Pointer;
use jsonld::{expand, JsonLdOptions};
use kroeg_tap::{
    as2, assign_ids, kroeg, ldp, untangle, Context, DefaultAuthorizer, MessageHandler, QuadQuery,
    QueryId, QueryObject, StoreError,
};
use kroeg_tab_activitypub::handlers;
use serde_json;
use std::collections::HashSet;

use crate::context::{self, SurfContextLoader};
use crate::request::store_all;
use crate::router::RequestHandler;
use crate::ServerError;

async fn prepare_delivery(
    context: &mut Context<'_, '_>,
    id: String,
    local: bool,
) -> Result<(), StoreError> {
    let obj = match context.entity_store.get(id.clone(), false).await? {
        Some(val) => val,
        None => return Ok(()),
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

    let follower_id = match context
        .entity_store
        .get(context.user.subject.to_owned(), true)
        .await?
    {
        Some(user) => {
            if let [Pointer::Id(id)] = &user.main()[as2!(followers)] as &[Pointer] {
                Some(id.to_owned())
            } else {
                None
            }
        }

        None => None,
    };

    while let Some((depth, item, is_shared)) = audience.pop() {
        let item = context.entity_store.get(item, false).await?;

        let item = if let Some(item) = item {
            item
        } else {
            continue;
        };

        // If this is a remote object, we will try to resolve shared inbox if available and allowed.
        // Local objects it makes no sense to do sharedInbox, as we will end up here again.
        // We will resolve local collections though!
        if !item.is_owned(context) {
            if local {
                let query = context
                    .entity_store
                    .query(vec![QuadQuery(
                        QueryId::Placeholder(0),
                        QueryId::Value(String::from(as2!(followers))),
                        QueryObject::Id(QueryId::Value(item.id().to_owned())),
                    )])
                    .await?;

                for mut item in query {
                    let user_id = match item.pop() {
                        Some(user_id) => user_id,
                        None => continue,
                    };

                    let result = context
                        .entity_store
                        .read_collection_inverse(user_id)
                        .await?;
                    let query = context
                        .entity_store
                        .query(vec![
                            QuadQuery(
                                QueryId::Placeholder(0),
                                QueryId::Value(String::from(as2!(following))),
                                QueryObject::Id(QueryId::Any(result.items)),
                            ),
                            QuadQuery(
                                QueryId::Placeholder(0),
                                QueryId::Value(String::from(ldp!(inbox))),
                                QueryObject::Id(QueryId::Placeholder(1)),
                            ),
                        ])
                        .await?;

                    for mut item in query {
                        boxes.insert(item.remove(1));
                    }
                }
            } else {
                let mut has_shared = false;

                if is_shared {
                    'outer: for endpoint in &item.main()[as2!(endpoints)] {
                        if let Pointer::Id(endpoint) = endpoint {
                            let item = context.entity_store.get(endpoint.clone(), true).await?;
                            if let Some(item) = item {
                                for inbox in &item.main()[as2!(sharedInbox)] {
                                    if let Pointer::Id(inbox) = inbox {
                                        boxes.insert(inbox.clone());
                                        has_shared = true;
                                        break 'outer;
                                    }
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
                    .iter()
                    .any(|f| f == as2!(OrderedCollection))
                    && depth < 3
                {
                    let data = context
                        .entity_store
                        .read_collection(item.id().to_owned(), Some(99999999), None)
                        .await?;

                    for fitem in data.items {
                        audience.push((
                            depth + 1,
                            fitem,
                            follower_id.as_ref().map(|f| f == item.id()) == Some(true),
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

    for send_to in boxes {
        let formatted = format!("{} {}", obj.id(), send_to);
        context
            .queue_store
            .add("deliver".to_owned(), formatted)
            .await?;
    }

    Ok(())
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

fn get_handler(box_type: &str) -> Option<(Vec<Box<dyn MessageHandler>>, DeliveryMode, TrustMode)> {
    match box_type {
        ldp!(inbox) => Some((
            vec![
                Box::new(handlers::VerifyRequiredEventsHandler(false)),
                Box::new(handlers::ServerCreateHandler),
                Box::new(handlers::ServerLikeHandler),
                Box::new(handlers::ServerFollowHandler),
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

        as2!(sharedInbox) => Some((
            vec![Box::new(handlers::VerifyRequiredEventsHandler(false))],
            DeliveryMode::LocalOnly,
            TrustMode::TrustIDs,
        )),

        _ => None,
    }
}

fn not_found() -> Response {
    http::Response::builder()
        .status(404)
        .body(Body::from(
            serde_json::json!({
                "@type": "https://puckipedia.com/kroeg/ns#NotFound",
                as2!(content): "Not found"
            })
            .to_string(),
        ))
        .unwrap()
}
pub struct PostHandler;

#[async_trait::async_trait]
impl RequestHandler for PostHandler {
    async fn run(
        &self,
        context: &mut Context<'_, '_>,
        request: Request,
    ) -> Result<Response, ServerError> {
        let id = format!("{}{}", context.server_base, request.uri().path());

        let (_, body) = request.into_parts();
        let body = body
            .into_vec()
            .await
            .map_err(|f| ServerError::HandlerError(f.into()))?;
        let json = serde_json::from_slice(&body).map_err(ServerError::SerdeError)?;

        let expanded = expand::<SurfContextLoader>(
            &context::apply_supplement(json),
            &JsonLdOptions {
                base: None,
                compact_arrays: None,
                expand_context: None,
                processing_mode: None,
            },
        )
        .await
        .map_err(ServerError::ExpansionError)?;

        let mut root = if let Some([serde_json::Value::Object(objdata)]) =
            expanded.as_array().map(|f| f as &[_])
        {
            objdata
                .get("@id")
                .and_then(|f| f.as_str())
                .map(|f| f.to_owned())
        } else {
            return Ok(not_found());
        };

        let inbox = context
            .entity_store
            .get(id, true)
            .await
            .map_err(ServerError::StoreError)?;

        let mut inbox = if let Some(inbox) = inbox {
            inbox
        } else {
            return Ok(not_found());
        };

        let box_type = if let [Pointer::Id(id)] = &inbox.meta()[kroeg!(box)] as &[Pointer] {
            id
        } else {
            return Err(ServerError::PostToNonbox);
        };

        let (handlers, delivery_mode, trust_mode) =
            get_handler(box_type).ok_or(ServerError::PostToNonbox)?;

        let mut untangled = untangle(&expanded).unwrap();

        if let TrustMode::TrustIDs = trust_mode {
            // We are posting to an inbox (aka server-to-server). This means we can
            //  trust anything that is on the same origin as the authorized user.

            let user: Uri = context.user.subject.parse().unwrap();
            let authority = user.authority_part().cloned();

            untangled.retain(|key, _| {
                let key = if key.starts_with("_:") {
                    &key[2..]
                } else {
                    key
                }
                .parse::<Uri>()
                .ok();

                key.and_then(|f| f.authority_part().cloned()) == authority
            });
        } else {
            // On outboxes, however, we use any external IDs, but ignore any internal IDs,
            //  and assign our own.

            root = assign_ids(
                context,
                Some(context.user.subject.to_owned()),
                &mut untangled,
                root,
            )
            .await
            .map_err(ServerError::StoreError)?;
        }

        store_all(
            context.entity_store,
            &DefaultAuthorizer,
            untangled.into_iter().map(|(_, v)| v).collect(),
        )
        .await
        .map_err(ServerError::StoreError)?;
        let mut user_inoutbox = inbox.id().to_owned();
        let mut root = root.unwrap();

        for handler in handlers {
            handler
                .handle(context, &mut user_inoutbox, &mut root)
                .await
                .map_err(ServerError::HandlerError)?;
        }

        match delivery_mode {
            DeliveryMode::None => Ok(()),
            DeliveryMode::LocalAndRemote => prepare_delivery(context, root.clone(), false).await,
            DeliveryMode::LocalOnly => prepare_delivery(context, root.clone(), true).await,
        }
        .map_err(ServerError::StoreError)?;

        context
            .entity_store
            .insert_collection(user_inoutbox, root.clone())
            .await
            .map_err(ServerError::StoreError)?;

        Ok(http::Response::builder()
            .status(201)
            .header("Location", &root)
            .header(
                "Content-Type",
                "application/ld+json; profile=\"https://www.w3.org/ns/activitystreams\"",
            )
            .body(Body::from(serde_json::json!({ "@id": root }).to_string()))
            .unwrap())
    }
}
