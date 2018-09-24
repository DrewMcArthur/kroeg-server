use futures::{
    future,
    future::Either,
    prelude::{await, *},
};

use super::context;
use context::HyperContextLoader;
use hyper;
use hyper::{Body, StatusCode, Uri};
use jsonld::{error::ExpansionError, expand, JsonLdOptions};
use kroeg_tap::{untangle, CollectionPointer, EntityStore, StoreItem};
use request::HyperLDRequest;
use serde_json::{from_slice, Error as SerdeError};
use std::collections::HashMap;
use std::error;
use std::fmt;
use std::sync::{Arc, Mutex};

#[derive(Debug)]
pub struct RetrievingEntityStore<T: EntityStore>(Arc<Mutex<T>>, String);

#[derive(Debug)]
pub enum RetrievingEntityStoreError<T: EntityStore> {
    HyperError(hyper::Error),
    SerdeError(SerdeError),
    StoreError(T::Error),
    ExpansionError(ExpansionError<context::HyperContextLoader>),
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
            RetrievingEntityStoreError::SerdeError(e) => write!(f, "Serde error: {}", e),
            RetrievingEntityStoreError::StoreError(e) => write!(f, "Store error: {}", e),
            RetrievingEntityStoreError::ExpansionError(e) => write!(f, "Expansion error: {}", e),
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
    store: Arc<Mutex<T>>,
    items: HashMap<String, StoreItem>,
) -> Result<Arc<Mutex<T>>, T::Error> {
    for (key, value) in items {
        await!(store.lock().unwrap().put(key, value))?;
    }

    Ok(store)
}

fn expand_and_unflatten<T: EntityStore>(
    id: String,
    body: Body,
) -> impl Future<Item = HashMap<String, StoreItem>, Error = RetrievingEntityStoreError<T>> {
    let authority = id.parse::<Uri>().ok().map(|f| f.authority_part().cloned());

    body.concat2()
        .map_err(RetrievingEntityStoreError::HyperError)
        .and_then(|value| {
            if value.as_ref().len() == 0 {
                Either::A(future::ok(json!([])))
            } else {
                Either::B(
                    from_slice(value.as_ref())
                        .map_err(RetrievingEntityStoreError::SerdeError)
                        .into_future(),
                )
            }
        }).and_then(|value| {
            expand::<HyperContextLoader>(
                context::apply_supplement(value),
                JsonLdOptions {
                    base: None,
                    compact_arrays: None,
                    expand_context: None,
                    processing_mode: None,
                },
            ).map_err(RetrievingEntityStoreError::ExpansionError)
        }).map(move |value| {
            let mut value = untangle(value).unwrap();
            value.retain(|k, _| {
                k.parse::<Uri>().ok().map(|f| f.authority_part().cloned()) == authority
            });
            value
        })
}

fn retrieve_and_store<T: EntityStore>(
    item: String,
    store: Arc<Mutex<T>>,
) -> impl Future<Item = Arc<Mutex<T>>, Error = RetrievingEntityStoreError<T>> {
    HyperLDRequest::new(&item)
        .map_err(RetrievingEntityStoreError::HyperError)
        .and_then(move |res| {
            if let Some(res) = res {
                if res.status() != StatusCode::OK {
                    Either::B(future::ok(HashMap::new()))
                } else {
                    Either::A(expand_and_unflatten(item, res.into_body()))
                }
            } else {
                Either::B(future::ok(HashMap::new()))
            }
        }).and_then(move |res| store_all(store, res).map_err(RetrievingEntityStoreError::StoreError))
}

impl<T: EntityStore> EntityStore for RetrievingEntityStore<T> {
    type Error = RetrievingEntityStoreError<T>;
    type GetFuture = Box<Future<Item = Option<StoreItem>, Error = Self::Error> + 'static + Send>;
    type StoreFuture = Box<Future<Item = StoreItem, Error = Self::Error> + 'static + Send>;
    type ReadCollectionFuture =
        Box<Future<Item = CollectionPointer, Error = Self::Error> + 'static + Send>;
    type WriteCollectionFuture = Box<Future<Item = (), Error = Self::Error> + 'static + Send>;

    fn get(&self, path: String, local: bool) -> Self::GetFuture {
        let future = self
            .0
            .lock()
            .unwrap()
            .get(path.to_owned(), local)
            .map_err(RetrievingEntityStoreError::StoreError);
        let store_copy = self.0.clone();
        let base = self.1.to_owned();

        Box::new(if local {
            Either::A(future)
        } else {
            Either::B(future.and_then(move |item| {
                if let Some(item) = item {
                    Either::A(future::ok(Some(item)))
                } else {
                    if path.starts_with(&base)
                        || path.starts_with("_:")
                        || path.starts_with(as2!(tag))
                    {
                        return Either::A(future::ok(None));
                    }

                    if path == as2!(Public) {
                        return Either::A(future::ok(
                            StoreItem::parse(
                                as2!(Public),
                                json!({
                                "@id": as2!(Public),
                                "@type": [as2!(Collection)]
                            }),
                            ).ok(),
                        ));
                    }

                    Either::B(retrieve_and_store(path.to_owned(), store_copy).and_then(
                        move |store| {
                            store
                                .lock()
                                .unwrap()
                                .get(path.to_owned(), true)
                                .map_err(RetrievingEntityStoreError::StoreError)
                        },
                    ))
                }
            }))
        })
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
