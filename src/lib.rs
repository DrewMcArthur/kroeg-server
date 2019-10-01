mod authentication;
pub mod config;
pub mod context;
pub mod delivery;
pub mod get;
pub mod jwt;
pub mod post;
pub mod request;
pub mod router;
mod store;
pub mod webfinger;

use http::StatusCode;
use http_service::{Body, HttpService, Request, Response};
use jsonld::error::{CompactionError, ExpansionError};
use kroeg_cellar::{CellarConnection, CellarEntityStore};
use kroeg_tap::{Context, StoreError};
use std::error::Error;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::store::RetrievingEntityStore;

#[derive(Debug)]
pub enum ServerError {
    HttpError(surf::Exception),
    SerdeError(serde_json::Error),
    StoreError(StoreError),
    ExpansionError(ExpansionError<context::SurfContextLoader>),
    CompactionError(CompactionError<context::SurfContextLoader>),
    HandlerError(Box<dyn Error + Send + Sync + 'static>),
    PostToNonbox,
    BadSharedInbox,
    Test,
}

impl fmt::Display for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ServerError::HttpError(err) => write!(f, "web client error: {}", err),
            ServerError::SerdeError(err) => write!(f, "serde error: {}", err),
            ServerError::StoreError(err) => write!(f, "store error: {}", err),
            ServerError::ExpansionError(err) => write!(f, "expansion error: {}", err),
            ServerError::CompactionError(err) => write!(f, "compaction error: {}", err),
            ServerError::HandlerError(err) => write!(f, "handler error: {}", err),
            ServerError::Test => write!(f, "Test!\n"),
            ServerError::PostToNonbox => write!(f, "tried to POST to a non-inbox/outbox entity"),
            ServerError::BadSharedInbox => {
                write!(f, "Mastodon quirk: actor != authorization token")
            }
        }
    }
}

impl Error for ServerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            ServerError::HttpError(err) => Some(err.as_ref()),
            ServerError::SerdeError(err) => Some(err),
            ServerError::StoreError(err) => Some(err.as_ref()),
            ServerError::ExpansionError(err) => Some(err),
            ServerError::CompactionError(err) => Some(err),
            _ => None,
        }
    }
}

/// The main service struct for Kroeg.
/// For each handled request, an instance of this struct is created by the KroegServiceBuilder.
/// This struct knows how to talk to the database, and has a list of routes.
#[derive(Clone)]
pub struct KroegService {
    data: Arc<(config::Config, Vec<router::Route>)>,
}

impl KroegService {
    pub fn new(config: config::Config, routes: Vec<router::Route>) -> KroegService {
        KroegService {
            data: Arc::new((config, routes)),
        }
    }
}

/// Helper function, that allows the router to fall back to 404 easily.
async fn not_found(
    _context: &mut Context<'_, '_>,
    _request: Request,
) -> Result<Response, ServerError> {
    Ok(http::Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Body::from("OwO I don't know what this is"))
        .unwrap())
}

/// Launches a delivery task.
pub async fn launch_delivery(config: config::Config) {
    loop {
        let db = &config.database;
        let conn = CellarConnection::connect(&db.server, &db.database, &db.username, &db.password)
            .await
            .unwrap();

        let mut entity_store = RetrievingEntityStore::new(
            CellarEntityStore::new(&conn),
            config.server.base_uri.to_owned(),
        );
        let mut queue_store = CellarEntityStore::new(&conn);

        let mut context = Context {
            server_base: config.server.base_uri.to_owned(),
            instance_id: config.server.instance_id,
            user: authentication::anonymous(),

            entity_store: &mut entity_store,
            queue_store: &mut queue_store,
        };

        if let Err(e) = delivery::loop_deliver(&mut context).await {
            println!(" - delivery thread failed: {:?}", e);
        }
    }
}

impl HttpService for KroegService {
    type Connection = ();
    type ConnectionFuture = Pin<Box<dyn Future<Output = Result<(), std::io::Error>> + Send>>;
    type ResponseFuture = Pin<Box<dyn Future<Output = Result<Response, std::io::Error>> + Send>>;

    fn connect(&self) -> Self::ConnectionFuture {
        Box::pin(async move { Ok(()) })
    }

    fn respond(&self, _: &mut (), req: http_service::Request) -> Self::ResponseFuture {
        let ptr = self.data.clone();

        Box::pin(async move {
            let (parts, body) = req.into_parts();

            let db = &ptr.0.database;
            let conn =
                CellarConnection::connect(&db.server, &db.database, &db.username, &db.password)
                    .await
                    .unwrap();

            let mut entity_store = RetrievingEntityStore::new(
                CellarEntityStore::new(&conn),
                ptr.0.server.base_uri.to_owned(),
            );
            let mut queue_store = CellarEntityStore::new(&conn);

            let user = match authentication::user_from_request(&parts, &mut entity_store).await {
                Ok(user) => user,
                Err(e) => {
                    println!(" - err setting up request: {:?}", e);

                    return Ok(http::Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Body::from(e.to_string()))
                        .unwrap());
                }
            };

            println!(" - {} {} ({:?})", parts.method, parts.uri, user.subject);

            let mut context = Context {
                server_base: ptr.0.server.base_uri.to_owned(),
                instance_id: ptr.0.server.instance_id,
                entity_store: &mut entity_store,
                queue_store: &mut queue_store,
                user,
            };

            let response = if let Some(route) = ptr.1.iter().rev().find(|f| f.can_handle(&parts)) {
                route
                    .handler
                    .run(&mut context, Request::from_parts(parts, body))
                    .await
            } else {
                not_found(&mut context, Request::from_parts(parts, body)).await
            };

            match response {
                Ok(response) => {
                    println!(" -   {}", response.status());

                    Ok(response)
                }

                Err(ServerError::HandlerError(e)) => {
                    println!(" -    handler err {:?}", e);

                    Ok(http::Response::builder()
                        .status(202)
                        .body(Body::from(e.to_string()))
                        .unwrap())
                }

                Err(e) => {
                    println!(" [ ] misc err {:?}", e);

                    Ok(http::Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Body::from(e.to_string()))
                        .unwrap())
                }
            }
        })
    }
}
