use kroeg_tap::{Authorizer, EntityStore, StoreError, StoreItem};
use serde_json::{json, Value};
use url::Url;

pub async fn do_request(url: &str) -> Result<Value, StoreError> {
    let mut url: Url = url.parse()?;

    for _ in 0..3usize {
        let mut response = surf::get(url.to_string())
            .set_header("Accept", "application/ld+json; profile=\"https://www.w3.org/ns/activitystreams\", application/activity+json, application/json")
            .await?;

        if let Some(header) = response.header("Location") {
            url = url.join(header)?;
            continue;
        }

        return match response.body_json().await {
            Ok(data) => Ok(data),
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => Ok(json!({})),
            Err(e) => Err(e.into()),
        };
    }

    Err("timed out".into())
}

pub async fn store_all(
    store: &mut dyn EntityStore,
    authorizer: &impl Authorizer,
    items: Vec<StoreItem>,
) -> Result<(), StoreError> {
    for mut item in items {
        match store.get(item.id().to_owned(), false).await? {
            Some(prev) => {
                if authorizer.can_replace(&prev, &item) {
                    store.put(item.id().to_owned(), &mut item).await?;
                }
            }

            None => {
                store.put(item.id().to_owned(), &mut item).await?;
            }
        }
    }

    Ok(())
}
