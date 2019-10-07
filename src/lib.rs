mod authentication;
pub mod config;
pub mod context;
pub mod delivery;
pub mod get;
pub mod jwt;
pub mod nodeinfo;
pub mod post;
pub mod request;
pub mod router;
pub mod store;
pub mod webfinger;

use http::StatusCode;
use http_service::{Body, HttpService, Request, Response};
use jsonld::error::{CompactionError, ExpansionError};
use kroeg_tap::{Context, EntityStore, QueueStore, StoreError};
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

/// A store connection pool.
pub trait StorePool: Send + Sync + 'static {
    type LeasedConnection: LeasedConnection;

    fn connect(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Self::LeasedConnection, StoreError>> + Send + 'static>>;
}

pub trait LeasedConnection: Send {
    fn get(&mut self) -> (&mut dyn EntityStore, &mut dyn QueueStore);
}

/// The main service struct for Kroeg.
/// For each handled request, an instance of this struct is created by the KroegServiceBuilder.
/// This struct knows how to talk to the database, and has a list of routes.
#[derive(Clone)]
pub struct KroegService<T: StorePool>(Arc<(T, config::ServerConfig, Vec<router::Route>)>);

impl<T: StorePool> KroegService<T> {
    pub fn new(
        store_pool: T,
        config: config::ServerConfig,
        routes: Vec<router::Route>,
    ) -> KroegService<T> {
        KroegService(Arc::new((store_pool, config, routes)))
    }
}

/// Launches a delivery task.
pub async fn launch_delivery<T: StorePool>(pool: T, config: config::ServerConfig) {
    loop {
        let mut pool = pool.connect().await.unwrap();

        let (entity_store, queue_store) = pool.get();
        let mut entity_store = RetrievingEntityStore::new(entity_store, config.domain.to_owned());

        let mut context = Context {
            server_base: config.domain.to_owned(),
            name: config.name.to_owned(),
            description: config.description.to_owned(),
            instance_id: config.instance_id,
            user: authentication::anonymous(),

            entity_store: &mut entity_store,
            queue_store,
        };

        if let Err(e) = delivery::loop_deliver(&mut context).await {
            println!(" - delivery thread failed: {:?}", e);
        }
    }
}

impl<T: StorePool> HttpService for KroegService<T> {
    type Connection = ();
    type ConnectionFuture = Pin<Box<dyn Future<Output = Result<(), std::io::Error>> + Send>>;
    type ResponseFuture = Pin<Box<dyn Future<Output = Result<Response, std::io::Error>> + Send>>;

    fn connect(&self) -> Self::ConnectionFuture {
        Box::pin(async move { Ok(()) })
    }

    fn respond(&self, _: &mut (), req: http_service::Request) -> Self::ResponseFuture {
        let ptr = self.0.clone();

        Box::pin(async move {
            let (parts, body) = req.into_parts();
            let response = async move {
                let mut database = ptr.0.connect().await.map_err(ServerError::StoreError)?;

                let (entity_store, queue_store) = database.get();

                let mut entity_store =
                    RetrievingEntityStore::new(entity_store, ptr.1.domain.to_owned());

                let user = match authentication::user_from_request(&parts, &mut entity_store).await
                {
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
                    server_base: ptr.1.domain.to_owned(),
                    name: ptr.1.name.to_owned(),
                    description: ptr.1.description.to_owned(),
                    instance_id: ptr.1.instance_id,
                    entity_store: &mut entity_store,
                    queue_store,
                    user,
                };

                if let Some(route) = ptr.2.iter().rev().find(|f| f.can_handle(&parts)) {
                    route
                        .handler
                        .run(&mut context, Request::from_parts(parts, body))
                        .await
                } else {
                    router::not_found(&mut context, Request::from_parts(parts, body)).await
                }
            }
                .await;

            match response {
                Ok(response) => {
                    println!("     {}", response.status());

                    Ok(response)
                }

                Err(ServerError::HandlerError(e)) => {
                    println!("      handler err {:?}", e);

                    Ok(http::Response::builder()
                        .status(202)
                        .body(Body::from(e.to_string()))
                        .unwrap())
                }

                Err(e) => {
                    println!("      misc err {:?}", e);

                    Ok(http::Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Body::from(e.to_string()))
                        .unwrap())
                }
            }
        })
    }
}
