use futures::{
    future,
    prelude::{await, *},
};

use context::HyperContextLoader;
use hyper;
use hyper::{Body, Client, Error, Request, Response, StatusCode, Uri};
use hyper_tls::HttpsConnector;
use jsonld::RemoteContextLoader;
use jsonld::{expand, JsonLdOptions};
use kroeg_tap::Context;
use kroeg_tap::{untangle, CollectionPointer, EntityStore, StoreItem};
use serde_json::{from_slice, Value};
use std::collections::HashMap;
use std::error;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::prelude::*;

#[derive(Debug)]
pub struct RetrievingEntityStore<T: EntityStore>(Arc<Mutex<T>>, String);

#[derive(Debug)]
pub enum RetrievingEntityStoreError<T: EntityStore> {
    HyperError(hyper::Error),
    StoreError(T::Error),
    Rest,
}

impl<T: EntityStore> error::Error for RetrievingEntityStoreError<T> {
    fn cause(&self) -> Option<&error::Error> {
        None
    }
}

impl<T: EntityStore> fmt::Display for RetrievingEntityStoreError<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            RetrievingEntityStoreError::HyperError(e) => write!(f, "Hyper error: {}", e),
            RetrievingEntityStoreError::StoreError(e) => write!(f, "Store error: {}", e),
            RetrievingEntityStoreError::Rest => write!(f, "unknown"),
        }
    }
}

impl<T: EntityStore> RetrievingEntityStore<T> {
    pub fn new(store: T, base: String) -> Self {
        RetrievingEntityStore(Arc::new(Mutex::new(store)), base)
    }

    pub fn unwrap(self) -> T {
        Arc::try_unwrap(self.0).unwrap().into_inner().unwrap()
    }
}

#[async]
fn store_all<T: EntityStore>(
    mut store: Arc<Mutex<T>>,
    items: HashMap<String, StoreItem>,
) -> Result<Arc<Mutex<T>>, T::Error> {
    for (key, value) in items {
        await!(store.lock().unwrap().put(key, value))?;
    }

    Ok(store)
}

impl<T: EntityStore> EntityStore for RetrievingEntityStore<T> {
    type Error = RetrievingEntityStoreError<T>;
    type GetFuture = Box<Future<Item = Option<StoreItem>, Error = Self::Error> + 'static + Send>;
    type StoreFuture = Box<Future<Item = StoreItem, Error = Self::Error> + 'static + Send>;
    type ReadCollectionFuture =
        Box<Future<Item = CollectionPointer, Error = Self::Error> + 'static + Send>;
    type WriteCollectionFuture = Box<Future<Item = (), Error = Self::Error> + 'static + Send>;

    fn get(&self, path: String, local: bool) -> Self::GetFuture {
        let base = self.1.to_owned();
        let clonerc = self.0.clone();
        Box::new(
            self.0
                .lock()
                .unwrap()
                .get(path.to_owned(), local)
                .map_err(RetrievingEntityStoreError::StoreError)
                .and_then(move |value| {
                    let val: Box<
                        Future<Item = Option<StoreItem>, Error = RetrievingEntityStoreError<T>>
                            + 'static
                            + Send,
                    > = match value {
                        Some(val) => {
                            Box::new(future::ok::<_, RetrievingEntityStoreError<T>>(Some(val)))
                        }
                        None => {
                            if path.starts_with(&base) || path.starts_with("_:")
                                || path.starts_with("https://www.w3.org/ns/activitystreams#tag")
                                || local
                            {
                                Box::new(future::ok(None))
                            } else {
                                eprint!(" ┃ retrieving {}", path);
                                let path = if path == "https://www.w3.org/ns/activitystreams#Public"
                                {
                                    "https://gist.githubusercontent.com/puckipedia/cdca8b2b213e92640118f4a2fe451c74/raw/161cba71bef63c7219597528033a0a3d9d2a62e3/bad.json".to_owned()
                                } else {
                                    path.to_owned()
                                };
                                let request = Request::get(path.to_owned())
                                .header("Accept", "application/ld+json; profile=\"https://www.w3.org/ns/activitystreams\", application/activity+json, application/json")
                                .body(Body::default())
                                .unwrap();

                                let connector = HttpsConnector::new(1).unwrap();
                                let client = Client::builder().build::<_, Body>(connector);

                                Box::new(
                                    client
                                        .request(request)
                                        .and_then(|res| {
                                            let boxed: Box<Future<Item = Option<hyper::Chunk>, Error = hyper::Error> + 'static + Send> = if res.status() == StatusCode::OK {
                                            Box::new(res.into_body().concat2().map(|f| Some(f)))
                                          } else {
                                            Box::new(future::ok(None))
                                          };

                                            boxed
                                        })
                                        .deadline(Instant::now() + Duration::from_secs(15))
                                        .map_err(|f| {
                                            if f.is_elapsed() {
                                                eprintln!(" fail timeout");
                                                RetrievingEntityStoreError::Rest
                                            } else {
                                                eprintln!(" fail hyper");
                                                RetrievingEntityStoreError::HyperError(
                                                    f.into_inner().unwrap(),
                                                )
                                            }
                                        })
                                        .then(move |val| {
                                            let response: Box<Future<Item = Option<StoreItem>, Error = RetrievingEntityStoreError<T>> + 'static + Send> = match val { Ok(val) => if let Some(val) = val {
                                        eprintln!(" done");
                                        match from_slice(&val) { Ok(res) => {
                                            Box::new(expand::<HyperContextLoader>(
                                                res,
                                                JsonLdOptions {
                                                    base: None,
                                                    compact_arrays: None,
                                                    expand_context: None,
                                                    processing_mode: None,
                                                },
                                            ).map_err(|_| RetrievingEntityStoreError::Rest)
                                            .and_then(move |expanded| {
                                                let host = {
                                                    let host = path.parse::<Uri>().unwrap();
                                                    host.authority_part().unwrap().clone()
                                                };
                                                let root = expanded.as_array().unwrap()[0]
                                                    .as_object()
                                                    .unwrap()["@id"]
                                                    .as_str()
                                                    .unwrap()
                                                    .to_owned();
                                                let mut untangled = untangle(expanded).unwrap();
                                                untangled.retain(|k, v| {
                                                    k.parse::<Uri>().unwrap().authority_part().unwrap()
                                                    == &host
                                                });
                                                store_all(clonerc, untangled).map_err(RetrievingEntityStoreError::StoreError).map(|store| (path, store))
                                            })
                                            .and_then(move |(path, store)| {
                                                store.lock().unwrap().get(path, local).map_err(RetrievingEntityStoreError::StoreError)
                                            }))
                                        }
                                        Err(_) => Box::new(future::ok(None))
                                        }
                                    } else {
                                        Box::new(future::ok(None))
                                    }, Err(RetrievingEntityStoreError::Rest) => Box::new(future::ok(None)), Err(e) => Box::new(future::err(e)) };

                                            response
                                        }),
                                )
                            }
                        }
                    };
                    val
                }),
        )
    }

    fn put(&mut self, path: String, item: StoreItem) -> Self::StoreFuture {
        Box::new(
            self.0
                .lock()
                .unwrap()
                .put(path, item)
                .map_err(RetrievingEntityStoreError::StoreError),
        )
    }

    fn read_collection(
        &self,
        path: String,
        count: Option<u32>,
        cursor: Option<String>,
    ) -> Self::ReadCollectionFuture {
        Box::new(
            self.0
                .lock()
                .unwrap()
                .read_collection(path, count, cursor)
                .map_err(RetrievingEntityStoreError::StoreError),
        )
    }

    fn find_collection(&self, path: String, item: String) -> Self::ReadCollectionFuture {
        Box::new(
            self.0
                .lock()
                .unwrap()
                .find_collection(path, item)
                .map_err(RetrievingEntityStoreError::StoreError),
        )
    }

    fn insert_collection(&mut self, path: String, item: String) -> Self::WriteCollectionFuture {
        Box::new(
            self.0
                .lock()
                .unwrap()
                .insert_collection(path, item)
                .map_err(RetrievingEntityStoreError::StoreError),
        )
    }

    fn remove_collection(&mut self, path: String, item: String) -> Self::WriteCollectionFuture {
        Box::new(
            self.0
                .lock()
                .unwrap()
                .remove_collection(path, item)
                .map_err(RetrievingEntityStoreError::StoreError),
        )
    }
}