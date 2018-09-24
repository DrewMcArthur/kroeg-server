use futures::future;
use futures::prelude::*;

use chashmap::CHashMap;
use hyper::Error;

use jsonld::RemoteContextLoader;
use serde_json::{from_slice, Value};

lazy_static! {
    /// List of contexts that have already been read.
    static ref CONTEXT_MAP: CHashMap<String, Value> = CHashMap::new();
}

use super::super::request::HyperLDRequest;

#[derive(Debug)]
pub struct HyperContextLoader;

impl RemoteContextLoader for HyperContextLoader {
    type Error = Error;
    type Future = Box<Future<Item = Value, Error = Error> + Send>;

    fn load_context(url: String) -> Self::Future {
        if let Some(val) = CONTEXT_MAP.get(&url) {
            return Box::new(future::ok(val.clone()));
        }

        let future = HyperLDRequest::new(&url);

        Box::new(
            future
                .and_then(|res| res.unwrap().into_body().concat2())
                .map(move |val| {
                    let res: Value =
                        from_slice(val.as_ref()).expect("Failed to parse context as JSON");
                    eprintln!(" [ ] Loaded context at: {}", url);
                    CONTEXT_MAP.insert(url, res.to_owned());
                    res
                }),
        )
    }
}
