use http_service::{Body, Request, Response};
use kroeg_tap::Context;
use serde_json::json;

use crate::{router::RequestHandler, router::Route, ServerError};

struct NodeInfoHandler;

#[async_trait::async_trait]
impl RequestHandler for NodeInfoHandler {
    async fn run(
        &self,
        context: &mut Context<'_, '_>,
        _: Request,
    ) -> Result<Response, ServerError> {
        Ok(http::Response::builder()
            .status(200)
            .header("Content-Type", "application/json")
            .body(Body::from(
                json!({
                    "links": [{
                        "rel": "http://nodeinfo.diaspora.software/ns/schema/2.0",
                        "href": format!("{}/-/nodeinfo/2.0", context.server_base)
                    }]
                })
                .to_string(),
            ))
            .unwrap())
    }
}

struct NodeInfo20Handler;

#[async_trait::async_trait]
impl RequestHandler for NodeInfo20Handler {
    async fn run(
        &self,
        context: &mut Context<'_, '_>,
        _: Request,
    ) -> Result<Response, ServerError> {
        Ok(http::Response::builder()
            .status(200)
            .header("Content-Type", "application/json")
            .body(Body::from(
                json!({
                    "version": "2.0",
                    "software": {
                        "name": "kroeg",
                        "version": "nya :3"
                    },
                    "protocols": ["activitypub"],
                    "services": { "inbound": [], "outbound": [] },
                    "openRegistrations": false,
                    "usage": {
                        "users": {
                            "total": 1,
                            "activeHalfyear": 1,
                            "activeMonth": 1
                        },
                        "localPosts": 69,
                        "localComments": 69
                    },
                    "metadata": {
                        "nodeName": context.name,
                        "nodeDescription": context.description,
                        "features": [],
                    },
                })
                .to_string(),
            ))
            .unwrap())
    }
}

pub fn routes() -> Vec<Route> {
    vec![
        Route::get("/.well-known/nodeinfo", NodeInfoHandler),
        Route::get("/-/nodeinfo/2.0", NodeInfo20Handler),
    ]
}
