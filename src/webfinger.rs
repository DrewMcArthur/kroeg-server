use http::Uri;
use http_service::{Body, Request, Response};
use jsonld::nodemap::{Pointer, Value};
use kroeg_tap::{as2, kroeg, Context, QuadQuery, QueryId, QueryObject, StoreItem};
use serde_json::{json, Value as JValue};

use crate::{router::RequestHandler, router::Route, ServerError};

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

struct WebfingerHandler;

#[async_trait::async_trait]
impl RequestHandler for WebfingerHandler {
    async fn run(
        &self,
        context: &mut Context<'_, '_>,
        request: Request,
    ) -> Result<Response, ServerError> {
        let (request, _) = request.into_parts();

        let users = match extract_acct(request.uri.query().unwrap_or("")) {
            Some((user, _)) => context
                .entity_store
                .query(vec![
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
                ])
                .await
                .map_err(ServerError::StoreError)?,

            None => vec![],
        };

        let user = match users.get(0).and_then(|f| f.get(0)) {
            Some(item) => context
                .entity_store
                .get(item.to_owned(), true)
                .await
                .map_err(ServerError::StoreError)?,
            None => None,
        };

        let user = match user {
            Some(user) => user,
            None => {
                return Ok(http::Response::builder()
                    .status(404)
                    .body(Body::from("not found"))
                    .unwrap())
            }
        };

        let username = extract_username(&user).unwrap();
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

        Ok(http::Response::builder()
            .status(200)
            .header("Content-Type", "application/jrd+json")
            .body(Body::from(response.to_string()))
            .unwrap())
    }
}

pub fn routes() -> Vec<Route> {
    vec![Route::get("/.well-known/webfinger", WebfingerHandler)]
}
