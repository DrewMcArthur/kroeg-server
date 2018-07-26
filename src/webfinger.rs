use futures::prelude::*;

use hyper::{Body, Request, Response};
use jsonld::nodemap::Pointer;
use kroeg_tap::{assemble, Authorizer, Context, DefaultAuthorizer, EntityStore, StoreItem};
use serde_json::Value;
use std::collections::HashSet;

use super::ServerError;

#[derive(Debug, Serialize)]
struct Link {
    pub rel: String,
    #[serde(rename = "type")]
    pub typ: String,
    pub href: String,
}

#[derive(Debug, Serialize)]
struct WebfingerResponse {
    pub subject: String,
    pub aliases: Vec<String>,
    pub links: Vec<Link>,
}

#[async(boxed_send)]
pub fn process<T: EntityStore>(
    context: Context,
    mut store: T,
    req: Request<Body>,
) -> Result<(T, Response<Body>), ServerError<T>> {
    let query = req.uri().query().map(|f| f.to_string());
    if let Some(query) = query {
        let p = query.split(':').collect::<Vec<_>>()[1]
            .split('@')
            .collect::<Vec<_>>()[0]
            .to_owned();
        let hacky = format!("{}/~{}", context.server_base, p);
        let user: Option<StoreItem> =
            await!(store.get(hacky.to_owned())).map_err(ServerError::StoreError)?;
        if let Some(user) = user {
            let response = WebfingerResponse {
                subject: format!(
                    "acct:{}@{}",
                    p,
                    context.server_base.split("//").collect::<Vec<_>>()[1]
                ),
                aliases: vec![user.id().to_owned()],
                links: vec![Link {
                    rel: "self".to_owned(),
                    typ: "application/activity+json".to_owned(),
                    href: user.id().to_owned(),
                }],
            };

            use serde_json;
            let response = serde_json::to_string(&response).unwrap();
            println!(" ┗ responding with webfinger for {}", hacky);
            Ok((
                store,
                Response::builder()
                    .header("Content-Type", "application/json")
                    .status(200)
                    .body(Body::from(response))
                    .unwrap(),
            ))
        } else {
            println!(" ┗ 404 webfinger {}", hacky);
            Ok((
                store,
                Response::builder()
                    .status(404)
                    .body(Body::from("no idea"))
                    .unwrap(),
            ))
        }
    } else {
        println!(" ┗ no clue");
        let response = Response::builder()
            .status(404)
            .body(Body::from("nope"))
            .unwrap();
        Ok((store, response))
    }
}
