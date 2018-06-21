#![feature(proc_macro, generators)]

extern crate toml;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate lazy_static;

extern crate chashmap;
extern crate futures_await as futures;
extern crate hyper;
extern crate hyper_tls;
extern crate jsonld;
#[macro_use]
extern crate serde_json;

extern crate diesel;
extern crate dotenv;
extern crate kroeg_cellar;
#[macro_use]
extern crate kroeg_tap;
extern crate kroeg_tap_activitypub;

mod authentication;
mod config;
mod context;
mod get;
mod post;

use authentication::user_from_request;

use diesel::prelude::*;

use kroeg_cellar::QuadClient;
use kroeg_tap::{Context, EntityStore};

use jsonld::error::{CompactionError, ExpansionError};
use jsonld::{compact, JsonLdOptions};

use futures::future;
use futures::prelude::*;

use std::fs::File;
use std::io::Read;
use std::{error, fmt};

use hyper::service::{NewService, Service};
use hyper::{Body, Method, Request, Response, Server, StatusCode};

static MSG_INVALID_METHOD: &'static [u8] = b"Invalid method";

pub fn context_from_config<T>(config: &config::Config, req: &Request<T>) -> Context {
    Context {
        server_base: config.server.base_uri.to_owned(),
        instance_id: config.server.instance_id,

        user: user_from_request(config, req),
    }
}

#[derive(Debug)]
pub enum ServerError<T: EntityStore> {
    HyperError(hyper::Error),
    SerdeError(serde_json::Error),
    StoreError(T::Error),
    ExpansionError(ExpansionError<context::HyperContextLoader>),
    CompactionError(CompactionError<context::HyperContextLoader>),
    HandlerError(Box<error::Error + Send + Sync + 'static>),
    PostToNonbox,
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

#[async]
fn compact_with_context(
    context: Context,
    val: serde_json::Value,
) -> Result<(Context, serde_json::Value), CompactionError<context::HyperContextLoader>> {
    let val = await!(compact::<context::HyperContextLoader>(
        val,
        context::get_context(&context),
        JsonLdOptions {
            base: None,
            compact_arrays: Some(true),
            expand_context: None,
            processing_mode: None,
        }
    ))?;

    Ok((context, val))
}

#[async]
fn compact_result<T: EntityStore>(
    context: Context,
    res: Response<serde_json::Value>,
) -> Result<(Context, Response<Body>), ServerError<T>> {
    let (parts, val) = res.into_parts();
    let (context, val) =
        await!(compact_with_context(context, val)).map_err(|e| ServerError::CompactionError(e))?;
    Ok((
        context,
        Response::from_parts(parts, Body::from(val.to_string())),
    ))
}

fn process_request<T: EntityStore>(
    context: Context,
    store: T,
    req: Request<Body>,
) -> Box<Future<Item = (T, Response<Body>), Error = ServerError<T>> + Send> {
    let future: Box<
        Future<Item = (T, Response<serde_json::Value>), Error = ServerError<T>> + Send,
    > = match *req.method() {
        Method::GET => {
            if req.uri().path() == "/_/context" {
                return Box::new(future::ok((store, context::extra_context())));
            }
            Box::new(get::process(context.clone(), store, req))
        }
        Method::POST => Box::new(post::process(context.clone(), store, req)),
        _ => {
            let body = Body::from(MSG_INVALID_METHOD);
            return Box::new(future::ok((
                store,
                Response::builder()
                    .status(StatusCode::METHOD_NOT_ALLOWED)
                    .body(body)
                    .unwrap(),
            )));
        }
    };

    Box::new(future.and_then(|(store, res)| {
        println!("{:?}", res);
        compact_result(context, res).map(move |(_, res)| (store, res))
    }))
}

#[derive(Clone)]
struct KroegService {
    config: config::Config,
}

impl Service for KroegService {
    type ReqBody = Body;
    type ResBody = Body;

    type Error = ServerError<QuadClient>;
    type Future =
        Box<Future<Item = Response<Body>, Error = ServerError<QuadClient>> + std::marker::Send>;

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let db = PgConnection::establish(&self.config.database)
            .expect(&format!("Error connecting to {}", self.config.database));
        let mut store = QuadClient::new(db);
        let in_transaction = *req.method() == Method::POST;
        if in_transaction {
            store.begin_transaction();
        }
        Box::new(
            process_request(context_from_config(&self.config, &req), store, req).then(move |x| {
                match x {
                    Ok((mut store, data)) => {
                        if in_transaction {
                            if data.status().is_success() {
                                store.commit_transaction();
                            } else {
                                store.rollback_transaction();
                            }
                        }

                        Ok(data)
                    }
                    Err(err) => Ok(Response::builder()
                        .status(500)
                        .body(Body::from(err.to_string()))
                        .unwrap()),
                }
            }),
        )
    }
}

struct KroegServiceBuilder {
    config: config::Config,
}

impl NewService for KroegServiceBuilder {
    type ReqBody = Body;
    type ResBody = Body;
    type Error = ServerError<QuadClient>;
    type Service = KroegService;
    type Future = future::FutureResult<KroegService, ServerError<QuadClient>>;
    type InitError = ServerError<QuadClient>;

    fn new_service(&self) -> future::FutureResult<KroegService, ServerError<QuadClient>> {
        future::ok(KroegService {
            config: self.config.clone(),
        })
    }
}

fn read_config() -> config::Config {
    let config_url = dotenv::var("CONFIG").unwrap_or("server.toml".to_owned());
    let mut file = File::open(&config_url).expect("Server config file not found!");
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).expect("Failed to read file!");

    toml::from_slice(&buffer).expect("Invalid config file!")
}

fn main() {
    dotenv::dotenv().ok();
    let config = read_config();

    let addr = &config.listen.parse().expect("Invalid listen address!");
    let server = Server::bind(&addr).serve(KroegServiceBuilder { config: config });

    println!("Kroeg v{} starting...", env!("CARGO_PKG_VERSION"));
    println!("listening at {}", addr);

    hyper::rt::run(server.map_err(|e| {
        eprintln!("Server error: {}", e);
    }))
}
