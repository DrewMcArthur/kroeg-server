use http::Uri;
use jsonld::{expand, JsonLdOptions};
use kroeg_tap::{
    as2, untangle, CollectionPointer, DefaultAuthorizer, EntityStore, QuadQuery, StoreError,
    StoreItem,
};
use serde_json::{json, Value};
use std::collections::HashMap;

use crate::context::{self, SurfContextLoader};
use crate::request::{do_request, store_all};

#[derive(Debug)]
pub struct RetrievingEntityStore<T>(T, String);

impl<T: EntityStore> RetrievingEntityStore<T> {
    pub fn new(store: T, base: String) -> Self {
        RetrievingEntityStore(store, base)
    }
}

async fn expand_and_unflatten(
    id: String,
    data: Value,
) -> Result<HashMap<String, StoreItem>, StoreError> {
    let authority = id.parse::<Uri>().ok().map(|f| f.authority_part().cloned());
    let expanded = expand::<SurfContextLoader>(
        &context::apply_supplement(data),
        &JsonLdOptions {
            base: None,
            compact_arrays: None,
            expand_context: None,
            processing_mode: None,
        },
    )
    .await?;

    let mut untangled = untangle(&expanded)?;
    untangled.retain(|key, _| {
        let my_authority = if key.starts_with("_:") {
            &key[2..]
        } else {
            key
        };

        my_authority
            .parse::<Uri>()
            .ok()
            .map(|f| f.authority_part().cloned())
            == authority
    });

    Ok(untangled)
}

async fn retrieve_and_store(item: String, store: &mut dyn EntityStore) -> Result<(), StoreError> {
    let response = do_request(&item).await?;
    let flattened = expand_and_unflatten(item, response).await?;

    store_all(
        store,
        &DefaultAuthorizer,
        flattened.into_iter().map(|(_, a)| a).collect(),
    )
    .await
}

#[async_trait::async_trait]
impl<T: EntityStore> EntityStore for RetrievingEntityStore<T> {
    async fn get(&mut self, path: String, local: bool) -> Result<Option<StoreItem>, StoreError> {
        if let Some(item) = self.0.get(path.clone(), local).await? {
            return Ok(Some(item));
        }

        if local {
            return Ok(None);
        }

        // Check for as:tag because `tag:abcd` will be deserialized as `as:tagabcd`. Fun.
        if path.starts_with("_:") || path.starts_with(&self.1) || path.starts_with(as2!(tag)) {
            return Ok(None);
        }

        if path == as2!(Public) {
            return Ok(StoreItem::parse(
                as2!(Public),
                &json!({
                    "@id": as2!(Public),
                    "@type": [as2!(Collection)]
                }),
            )
            .ok());
        }

        retrieve_and_store(path.clone(), &mut self.0).await?;

        self.0.get(path, local).await
    }

    async fn put(&mut self, path: String, item: &mut StoreItem) -> Result<(), StoreError> {
        self.0.put(path, item).await
    }

    async fn query(&mut self, query: Vec<QuadQuery>) -> Result<Vec<Vec<String>>, StoreError> {
        self.0.query(query).await
    }

    async fn read_collection(
        &mut self,
        path: String,
        count: Option<u32>,
        cursor: Option<String>,
    ) -> Result<CollectionPointer, StoreError> {
        self.0.read_collection(path, count, cursor).await
    }

    async fn find_collection(
        &mut self,
        path: String,
        item: String,
    ) -> Result<CollectionPointer, StoreError> {
        self.0.find_collection(path, item).await
    }

    async fn insert_collection(&mut self, path: String, item: String) -> Result<(), StoreError> {
        self.0.insert_collection(path, item).await
    }

    async fn read_collection_inverse(
        &mut self,
        item: String,
    ) -> Result<CollectionPointer, StoreError> {
        self.0.read_collection_inverse(item).await
    }

    async fn remove_collection(&mut self, path: String, item: String) -> Result<(), StoreError> {
        self.0.remove_collection(path, item).await
    }
}
