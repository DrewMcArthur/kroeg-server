use futures::future;
use futures::prelude::{await, *};

use chashmap::CHashMap;
use hyper::{Body, Client, Error, Request, Response};
use hyper_tls::HttpsConnector;
use jsonld::RemoteContextLoader;
use kroeg_tap::Context;
use serde_json::{from_slice, Value};

fn get_extra_context() -> Value {
    from_slice(include_bytes!("context.json")).unwrap()
}

pub fn extra_context() -> Response<Body> {
    Response::builder()
        .body(Body::from(get_extra_context().to_string()))
        .unwrap()
}

pub fn get_context(context: &Context) -> Value {
    Value::Array(vec![
        Value::String("https://www.w3.org/ns/activitystreams".to_owned()),
        Value::String(format!("{}/-/context", context.server_base)),
    ])
}

lazy_static! {
    static ref CONTEXT_MAP: CHashMap<String, Value> = CHashMap::new();
}

#[derive(Debug)]
pub struct HyperContextLoader;

impl RemoteContextLoader for HyperContextLoader {
    type Error = Error;
    type Future = Box<Future<Item = Value, Error = Error> + Send>;

    fn load_context(url: String) -> Self::Future {
        let url = if url == "https://w3id.org/security/v1" {
            "https://web-payments.org/contexts/security-v1.jsonld".to_owned()
        } else {
            url
        };

        if let Some(val) = CONTEXT_MAP.get(&url) {
            return Box::new(future::ok(val.clone()));
        }

        let request = Request::get(url.to_owned())
            .header("Accept", "application/ld+json; profile=\"https://www.w3.org/ns/activitystreams\", application/activity+json, application/json")
            .body(Body::default())
            .unwrap();

        let connector = HttpsConnector::new(1).unwrap();
        let client = Client::builder().build::<_, Body>(connector);

        Box::new(
            client
                .request(request)
                .and_then(|res| res.into_body().concat2())
                .map(move |val| {
                    let res: Value =
                        from_slice(val.as_ref()).expect("Failed to parse context as JSON");
                    eprintln!(" ┃ loaded context {}", url);
                    CONTEXT_MAP.insert(url, res.to_owned());
                    res
                }),
        )
    }
}
