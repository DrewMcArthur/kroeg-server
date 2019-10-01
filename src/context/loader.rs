use chashmap::CHashMap;
use jsonld::RemoteContextLoader;
use serde_json::Value;
use std::error::Error;
use std::fmt::{self, Display};
use std::future::Future;
use std::pin::Pin;

use crate::request::do_request;

lazy_static::lazy_static! {
    /// List of contexts that have already been read.
    static ref CONTEXT_MAP: CHashMap<String, Value> = CHashMap::new();
}

#[derive(Debug)]
pub struct SurfContextLoader;

#[derive(Debug)]
pub struct ContextLoadError(Box<dyn Error + Send + Sync + 'static>);
impl Error for ContextLoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.0.source()
    }
}

impl Display for ContextLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl RemoteContextLoader for SurfContextLoader {
    type Error = ContextLoadError;
    type Future = Pin<Box<dyn Future<Output = Result<Value, Self::Error>> + Send + 'static>>;

    fn load_context(url: String) -> Self::Future {
        Box::pin(async move {
            if let Some(val) = CONTEXT_MAP.get(&url) {
                return Ok(val.clone());
            }

            let response: Value = do_request(&url).await.map_err(ContextLoadError)?;
            eprintln!(" [ ] Loaded context at: {}", url);
            CONTEXT_MAP.insert(url, response.clone());

            Ok(response)
        })
    }
}
