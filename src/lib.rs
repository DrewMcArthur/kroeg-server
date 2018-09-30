#![feature(generators, use_extern_macros)]

#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate serde_json;
#[macro_use]
extern crate kroeg_tap;

extern crate base64;
extern crate chashmap;
extern crate diesel;
extern crate dotenv;
extern crate futures_await as futures;
extern crate http;
extern crate hyper;
extern crate hyper_tls;
extern crate jsonld;
extern crate kroeg_cellar;
extern crate kroeg_tap_activitypub;
extern crate openssl;
extern crate serde;
extern crate sha2;
extern crate tokio;
extern crate toml;

mod authentication;
pub mod config;
pub mod context;
pub mod delivery;
pub mod get;
mod jwt;
pub mod post;
pub mod request;
pub mod router;
mod store;
pub mod webfinger;

use diesel::prelude::*;
use futures::prelude::*;

use authentication::user_from_request;
use futures::future;
use hyper::service::{NewService, Service};
use hyper::{Body, Request, Response, StatusCode};
use jsonld::error::{CompactionError, ExpansionError};
use kroeg_cellar::QuadClient;
use kroeg_tap::{Context, EntityStore, QueueStore};
use router::Route;
use serde_json::Value;
use std::sync::Arc;
use std::{error, fmt};
use store::RetrievingEntityStore;

#[derive(Debug)]
pub enum ServerError<T: EntityStore> {
    HyperError(hyper::Error),
    SerdeError(serde_json::Error),
    StoreError(T::Error),
    ExpansionError(ExpansionError<context::HyperContextLoader>),
    CompactionError(CompactionError<context::HyperContextLoader>),
    HandlerError(Box<error::Error + Send + Sync + 'static>),
    PostToNonbox,
    BadSharedInbox,
    Test,
}

impl<T: EntityStore> fmt::Display for ServerError<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ServerError::HyperError(err) => write!(f, "hyper error: {}", err),
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

impl<T: EntityStore> error::Error for ServerError<T> {
    fn cause(&self) -> Option<&error::Error> {
        match self {
            ServerError::HyperError(err) => Some(err),
            ServerError::SerdeError(err) => Some(err),
            ServerError::StoreError(err) => Some(err),
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
    config: config::Config,
    routes: Vec<Route<RetrievingEntityStore<QuadClient>, QuadClient>>,
}

/// Helper function, that allows the router to fall back to 404 easily.
fn not_found<T: EntityStore, R: QueueStore>(
    _: Context,
    store: T,
    queue: R,
    _: Request<Body>,
) -> Box<Future<Item = (T, R, Response<Body>), Error = (ServerError<T>, T)> + Send> {
    let response = Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Body::from("unhandled route"))
        .unwrap();

    Box::new(future::ok((store, queue, response)))
}

pub fn launch_delivery(config: config::Config) -> impl Future<Item = (), Error = ()> + Send {
    let (store, queue) = match (
        PgConnection::establish(&config.database),
        PgConnection::establish(&config.database),
    ) {
        (Ok(store_db), Ok(queue_db)) => (
            RetrievingEntityStore::new(
                QuadClient::new(store_db),
                config.server.base_uri.to_owned(),
            ),
            QuadClient::new(queue_db),
        ),

        (Err(e), _) | (_, Err(e)) => {
            panic!("Database connection failed: {}", e);
        }
    };

    let context = Context {
        server_base: config.server.base_uri.to_owned(),
        instance_id: config.server.instance_id,
        user: authentication::anonymous(),
    };

    delivery::loop_deliver(context, store, queue)
}

impl Service for KroegService {
    type ReqBody = Body;
    type ResBody = Body;

    type Error = ServerError<RetrievingEntityStore<QuadClient>>;
    type Future = Box<Future<Item = Response<Body>, Error = Self::Error> + Send>;

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        // When an incoming request gets handled, the first thing needed is connections to the database.
        // For legacy design reasons, for now, we'll open two connections. One for the quad store, one for the queue.
        let (store, queue) = match (
            PgConnection::establish(&self.config.database),
            PgConnection::establish(&self.config.database),
        ) {
            (Ok(store_db), Ok(queue_db)) => (
                RetrievingEntityStore::new(
                    QuadClient::new(store_db),
                    self.config.server.base_uri.to_owned(),
                ),
                QuadClient::new(queue_db),
            ),

            (Err(e), _) | (_, Err(e)) => {
                eprintln!("Database connection failed: {}", e);

                return Box::new(future::ok(
                    Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Body::from("Database connection failed."))
                        .unwrap(),
                ));
            }
        };

        let routes = self.routes.clone();
        let (parts, body) = req.into_parts();

        Box::new(
            user_from_request(self.config.clone(), parts, store)
                .then(move |result| match result {
                    Ok((config, parts, store, user)) => {
                        let request = Request::from_parts(parts, body);

                        let context = Context {
                            server_base: config.server.base_uri.to_owned(),
                            instance_id: config.server.instance_id,
                            user: user,
                        };

                        let route = routes
                            .iter()
                            .rev()
                            .find(|f| f.can_handle(&request))
                            .map(|f| f.handler.clone())
                            .unwrap_or_else(|| Arc::new(Box::new(not_found)));

                        route(context, store, queue, request)
                    }

                    Err((e, store)) => Box::new(future::err((ServerError::StoreError(e), store))),
                })
                // Handlers return a tuple (EntityStore, QueueStore, Response) but we need to return a Response<Body>.
                // so, we drop the EntityStore and QueueStore. (note to self: bring transactions back)
                .then(|result| {
                    match result {
                        Ok((.., response)) => Ok(response),
                        Err((err, _)) => {
                            eprintln!("Error handling request: {}", err);

                            // If the Service returns an error, the connection with the client will just drop.
                            // This doesn't seem like the best way to go, so return a 500 instead.
                            Ok(Response::builder()
                                .status(StatusCode::INTERNAL_SERVER_ERROR)
                                .body(Body::from(format!("error: {}", err)))
                                .unwrap())
                        }
                    }
                }),
        )
    }
}

pub struct KroegServiceBuilder {
    pub config: config::Config,
    pub routes: Vec<Route<RetrievingEntityStore<QuadClient>, QuadClient>>,
}

impl NewService for KroegServiceBuilder {
    type ReqBody = Body;
    type ResBody = Body;
    type Error = ServerError<RetrievingEntityStore<QuadClient>>;
    type Service = KroegService;
    type Future = future::FutureResult<KroegService, Self::Error>;
    type InitError = Self::Error;

    fn new_service(&self) -> Self::Future {
        future::ok(KroegService {
            config: self.config.clone(),
            routes: self.routes.clone(),
        })
    }
}

pub fn compact_response<
    T: EntityStore,
    R: QueueStore,
    F: Future<Item = (T, R, Response<Value>), Error = (ServerError<T>, T)> + Send + 'static,
    S: 'static + Send + Sync + Fn(Context, T, R, Request<Body>) -> F,
>(
    function: S,
) -> router::RequestHandler<T, R> {
    Box::new(move |context, store, queue, request| {
        Box::new(
            function(context.clone(), store, queue, request)
                .and_then(move |(store, queue, response)| {
                    let (parts, body) = response.into_parts();

                    context::compact(&context, body)
                        .then(|val| match val {
                            Ok(val) => Ok((store, queue, parts, val)),
                            Err(e) => Err((ServerError::CompactionError(e), store)),
                        })
                }).map(|(store, queue, parts, body)| {
                    (
                        store,
                        queue,
                        Response::from_parts(parts, Body::from(body.to_string())),
                    )
                }),
        )
    })
}
