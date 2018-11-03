use futures::{future, future::Either, prelude::*};

use super::context;
use context::HyperContextLoader;
use hyper;
use hyper::{Body, StatusCode, Uri};
use jsonld::{error::ExpansionError, expand, JsonLdOptions};
use kroeg_tap::{untangle, CollectionPointer, EntityStore, QuadQuery, StoreItem};
use request::{HyperLDRequest, StoreAllFuture};
use serde_json::{from_slice, Error as SerdeError};
use std::collections::HashMap;
use std::error;
use std::fmt;

#[derive(Debug)]
pub struct RetrievingEntityStore<T: EntityStore>(T, String);

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
        RetrievingEntityStore(store, base)
    }

    pub fn unwrap(self) -> T {
        self.0
    }
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
        })
        .and_then(|value| {
            expand::<HyperContextLoader>(
                context::apply_supplement(value),
                JsonLdOptions {
                    base: None,
                    compact_arrays: None,
                    expand_context: None,
                    processing_mode: None,
                },
            )
            .map_err(RetrievingEntityStoreError::ExpansionError)
        })
        .map(move |value| {
            let mut value = untangle(value).unwrap();
            value.retain(|k, _| {
                k.parse::<Uri>().ok().map(|f| f.authority_part().cloned()) == authority
            });
            value
        })
}

fn retrieve_and_store<T: EntityStore>(
    item: String,
    store: T,
) -> impl Future<Item = T, Error = (RetrievingEntityStoreError<T>, T)> {
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
        })
        .then(move |res| match res {
            Ok(res) => Either::A(
                StoreAllFuture::new(store, res.into_iter().map(|(_, a)| a).collect())
                    .map_err(|(e, store)| (RetrievingEntityStoreError::StoreError(e), store)),
            ),
            Err(e) => Either::B(future::err((e, store))),
        })
}

fn make_retrieving<T, Q, E: EntityStore>(
    base: String,
) -> impl FnOnce(Result<(T, E), (Q, E)>)
    -> Result<(T, RetrievingEntityStore<E>), (Q, RetrievingEntityStore<E>)> {
    move |f| match f {
        Ok((item, store)) => Ok((item, RetrievingEntityStore(store, base))),
        Err((item, store)) => Err((item, RetrievingEntityStore(store, base))),
    }
}

fn make_retrieving_nop<Q, E: EntityStore>(
    base: String,
) -> impl FnOnce(Result<(E), (Q, E)>) -> Result<(RetrievingEntityStore<E>), (Q, RetrievingEntityStore<E>)>
{
    move |f| match f {
        Ok(store) => Ok(RetrievingEntityStore(store, base)),
        Err((item, store)) => Err((item, RetrievingEntityStore(store, base))),
    }
}

impl<T: EntityStore> EntityStore for RetrievingEntityStore<T> {
    type Error = RetrievingEntityStoreError<T>;
    type GetFuture =
        Box<Future<Item = (Option<StoreItem>, Self), Error = (Self::Error, Self)> + 'static + Send>;
    type StoreFuture =
        Box<Future<Item = (StoreItem, Self), Error = (Self::Error, Self)> + 'static + Send>;
    type ReadCollectionFuture =
        Box<Future<Item = (CollectionPointer, Self), Error = (Self::Error, Self)> + 'static + Send>;
    type WriteCollectionFuture =
        Box<Future<Item = Self, Error = (Self::Error, Self)> + 'static + Send>;
    type QueryFuture =
        Box<Future<Item = (Vec<Vec<String>>, Self), Error = (Self::Error, Self)> + 'static + Send>;

    fn get(self, path: String, local: bool) -> Self::GetFuture {
        let future = self
            .0
            .get(path.to_owned(), local)
            .map_err(|(e, store)| (RetrievingEntityStoreError::StoreError(e), store));
        let base = self.1.to_owned();
        Box::new(
            if local {
                Either::A(future)
            } else {
                Either::B(future.and_then(move |(item, store)| {
                    if let Some(item) = item {
                        Either::A(future::ok((Some(item), store)))
                    } else {
                        if path.starts_with(&base)
                            || path.starts_with("_:")
                            || path.starts_with(as2!(tag))
                        {
                            return Either::A(future::ok((None, store)));
                        }

                        if path == as2!(Public) {
                            return Either::A(future::ok((
                                StoreItem::parse(
                                    as2!(Public),
                                    json!({
                                    "@id": as2!(Public),
                                    "@type": [as2!(Collection)]
                                }),
                                )
                                .ok(),
                                store,
                            )));
                        }

                        Either::B(retrieve_and_store(path.to_owned(), store).and_then(
                            move |store| {
                                store.get(path.to_owned(), true).map_err(|(e, store)| {
                                    (RetrievingEntityStoreError::StoreError(e), store)
                                })
                            },
                        ))
                    }
                }))
            }
            .then(make_retrieving(self.1)),
        )
    }

    fn put(self, path: String, item: StoreItem) -> Self::StoreFuture {
        Box::new(
            self.0
                .put(path, item)
                .map_err(|(e, store)| (RetrievingEntityStoreError::StoreError(e), store))
                .then(make_retrieving(self.1)),
        )
    }

    fn query(self, query: Vec<QuadQuery>) -> Self::QueryFuture {
        Box::new(
            self.0
                .query(query)
                .map_err(|(e, store)| (RetrievingEntityStoreError::StoreError(e), store))
                .then(make_retrieving(self.1)),
        )
    }

    fn read_collection(
        self,
        path: String,
        count: Option<u32>,
        cursor: Option<String>,
    ) -> Self::ReadCollectionFuture {
        Box::new(
            self.0
                .read_collection(path, count, cursor)
                .map_err(|(e, store)| (RetrievingEntityStoreError::StoreError(e), store))
                .then(make_retrieving(self.1)),
        )
    }

    fn read_collection_inverse(self, item: String) -> Self::ReadCollectionFuture {
        Box::new(
            self.0
                .read_collection_inverse(item)
                .map_err(|(e, store)| (RetrievingEntityStoreError::StoreError(e), store))
                .then(make_retrieving(self.1)),
        )
    }

    fn find_collection(self, path: String, item: String) -> Self::ReadCollectionFuture {
        Box::new(
            self.0
                .find_collection(path, item)
                .map_err(|(e, store)| (RetrievingEntityStoreError::StoreError(e), store))
                .then(make_retrieving(self.1)),
        )
    }

    fn insert_collection(self, path: String, item: String) -> Self::WriteCollectionFuture {
        Box::new(
            self.0
                .insert_collection(path, item)
                .map_err(|(e, store)| (RetrievingEntityStoreError::StoreError(e), store))
                .then(make_retrieving_nop(self.1)),
        )
    }

    fn remove_collection(self, path: String, item: String) -> Self::WriteCollectionFuture {
        Box::new(
            self.0
                .remove_collection(path, item)
                .map_err(|(e, store)| (RetrievingEntityStoreError::StoreError(e), store))
                .then(make_retrieving_nop(self.1)),
        )
    }
}
