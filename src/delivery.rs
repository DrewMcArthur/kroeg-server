use base64;
use futures::prelude::{await, *};
use kroeg_tap::{assemble, Context, DefaultAuthorizer, EntityStore, QueueItem, QueueStore};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::time::{Duration, Instant};
use tokio::prelude::*;
use tokio::timer::Delay;

use context;
use hyper::client::HttpConnector;
use hyper::{header::HeaderValue, Body, Client, Method, Request};
use hyper_tls::HttpsConnector;
use jsonld::nodemap::Pointer;
use jsonld::{compact, error::CompactionError, JsonLdOptions};
use kroeg_tap::StoreItem;
use openssl::{hash::MessageDigest, pkey::PKey, rsa::Rsa, sign::Signer};
use serde_json::Value as JValue;

pub fn escape(s: &str) -> String {
    s.replace("\\", "\\\\").replace(" ", "\\s")
}

#[async]
pub fn compact_with_context(
    context: Context,
    val: JValue,
) -> Result<(Context, JValue), CompactionError<context::HyperContextLoader>> {
    let val = await!(compact::<context::HyperContextLoader>(
        val,
        context::outgoing_context(&context),
        JsonLdOptions {
            base: None,
            compact_arrays: Some(true),
            expand_context: None,
            processing_mode: None,
        }
    ))?;

    Ok((context, val))
}

pub fn create_signature(data: &str, key_object: &StoreItem, req: &mut Request<Body>) {
    let digest = Sha256::digest_str(data);
    let digest = base64::encode_config(&digest, base64::STANDARD);

    req.headers_mut()
        .insert("Digest", format!("SHA-256={}", digest).parse().unwrap());

    let pem_data = key_object.sub(kroeg!(meta)).unwrap()[sec!(privateKeyPem)]
        .iter()
        .next()
        .and_then(|f| match f {
            Pointer::Value(val) => {
                if let JValue::String(strval) = &val.value {
                    Some(strval.to_owned())
                } else {
                    None
                }
            }
            _ => None,
        })
        .and_then(|f| Rsa::private_key_from_pem(f.as_bytes()).ok())
        .unwrap();
    let key = PKey::from_rsa(pem_data).unwrap();
    let mut signer = Signer::new(MessageDigest::sha256(), &key).unwrap();

    let mut signed = String::new();
    let headers = &["(request-target)", "digest"];
    for val in headers {
        let value = match *val {
            "(request-target)" => format!(
                "(request-target): {} {}{}",
                req.method().as_str().to_lowercase(),
                req.uri().path(),
                match req.uri().query() {
                    None => format!(""),
                    Some(val) => format!("?{}", val),
                }
            ),

            val => format!(
                "{}: {}",
                val,
                req.headers()
                    .get_all(val)
                    .iter()
                    .map(|f| f.to_str().unwrap())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };

        if signed.len() > 0 {
            signed += "\n";
        }

        signed += &value;
    }

    signer.update(signed.as_bytes()).unwrap();
    let signature = base64::encode_config(&signer.sign_to_vec().unwrap(), base64::STANDARD);

    req.headers_mut().insert(
        "Signature",
        format!(
            "keyId=\"{}\",algorithm=\"rsa-sha256\",headers=\"{}\",signature=\"{}\"",
            key_object.id(),
            headers.join(" "),
            signature
        )
        .parse()
        .unwrap(),
    );
}

#[async]
pub fn deliver_one<T: EntityStore, R: QueueStore>(
    mut context: Context,
    client: Client<HttpsConnector<HttpConnector>, Body>,
    store: T,
    item: R::Item,
) -> Result<
    (
        Context,
        Client<HttpsConnector<HttpConnector>, Body>,
        T,
        R::Item,
    ),
    (
        Context,
        Client<HttpsConnector<HttpConnector>, Body>,
        T,
        R::Item,
        T::Error,
    ),
> {
    match item.event() {
        "deliver" => {
            println!(" [+] delivering {:?}", item.data());
            // bad. urlencode instead.
            let mut data: Vec<_> = item
                .data()
                .split(' ')
                .map(|f| f.replace("\\s", " ").replace("\\\\", "\\"))
                .collect();
            let uri = data.remove(1);
            let itemid = data.remove(0);

            let (sdata, store) = match await!(store.get(itemid, false)) {
                Ok((Some(ok), store)) => (ok, store),
                Ok((None, store)) => return Ok((context, client, store, item)),
                Err((err, store)) => return Err((context, client, store, item, err)),
            };

            if let Pointer::Id(id) = sdata.main()[as2!(actor)][0].to_owned() {
                context.user.subject = id;
            }

            let (_, nstore, _, data) = await!(assemble(
                sdata.clone(),
                0,
                Some(store),
                DefaultAuthorizer::new(&context),
                HashSet::new()
            ))
            .unwrap();

            let store = nstore.unwrap();

            let (context, data) = await!(compact_with_context(context, data)).unwrap();

            let (_, store) = match await!(store.get(uri.to_owned(), true)) {
                Ok((Some(val), store)) => (val.is_owned(&context), store),
                Ok((None, store)) => (false, store),
                Err((err, store)) => return Err((context, client, store, item, err)),
            };

            let (headers, store) = /*if is_local {
                /* post::process(context, store, queue, request); */
            } else */{
                let mut req = Request::new(Body::from(data.to_string()));
                *req.method_mut() = Method::POST;
                *req.uri_mut() = uri.parse().unwrap();
                req.headers_mut().insert("Content-Type", HeaderValue::from_str("application/ld+json; profile=\"https://www.w3.org/ns/activitystreams\"").unwrap());

                let (owner, store) = if let Pointer::Id(id) = sdata.main()[as2!(actor)][0].to_owned() {
                    match await!(store.get(id, false)) {
                        Ok((Some(val), store)) => (val, store),
                        _ => panic!("todo"),
                    }
                } else {
                    panic!("todo");
                };

                let (key_object, store) = if let Pointer::Id(id) = owner.main()[sec!(publicKey)][0].to_owned() {
                    match await!(store.get(id, false)) {
                        Ok((Some(val), store)) => (val, store),
                        _ => panic!("todo"),
                    }
                } else {
                    panic!("todo")
                };

                create_signature(&data.to_string(), &key_object, &mut req);

                let response = match await!(client.request(req).timeout(Duration::from_millis(10000))) { Ok(val) => val, Err(err) => { println!("ERR {:?}", err); return Ok((context, client, store, item)); }};
                let (header, _) = response.into_parts();

                (header, store)
            };

            println!(" [+] {} {}", uri, headers.status);

            Ok((context, client, store, item))
        }

        _ => {
            panic!("oh no");
        }
    }
}

#[async]
pub fn register_delivery<R: QueueStore>(
    queue: R,
    item: String,
    towards: String,
) -> Result<R, (R::Error, R)> {
    await!(queue.add(
        "deliver".to_owned(),
        format!("{} {}", escape(&item), escape(&towards))
    ))
}

#[async]
pub fn loop_deliver<T: EntityStore, R: QueueStore>(
    mut context: Context,
    mut store: T,
    mut queue: R,
) -> Result<(), ()> {
    println!("[+] Delivery thread start");
    let connector = HttpsConnector::new(1).unwrap();
    let mut client = Client::builder().build::<_, Body>(connector);

    loop {
        let (item, _queue) = await!(queue.get_item()).unwrap();
        queue = _queue;
        match item {
            Some(val) => {
                match await!(deliver_one::<T, R>(context, client, store, val)) {
                    Ok((co, cl, s, item)) => {
                        context = co;
                        client = cl;
                        store = s;
                        queue = await!(queue.mark_success(item)).unwrap();
                    }
                    Err((co, _cl, s, item, _)) => {
                        context = co;
                        // client = cl;
                        store = s;
                        queue = await!(queue.mark_failure(item)).unwrap();

                        return Err(());
                    }
                };
            }

            None => {
                await!(Delay::new(Instant::now() + Duration::from_millis(10000))).unwrap();
            }
        };
    }
}
