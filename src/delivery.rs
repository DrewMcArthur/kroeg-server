use futures::future::FutureExt;
use futures_timer::{Delay, TryFutureExt};
use http_service::Body;
use jsonld::nodemap::{Pointer, Value};
use jsonld::{compact, error::CompactionError, JsonLdOptions};
use kroeg_tap::{
    as2, assemble, kroeg, sec, Context, DefaultAuthorizer, LocalOnlyAuthorizer, QueueItem,
};
use kroeg_tap::{StoreError, StoreItem};
use openssl::{hash::MessageDigest, pkey::PKey, rsa::Rsa, sign::Signer};
use serde_json::{json, Value as JValue};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fmt::Debug;
use std::time::Duration;

use crate::context;
use crate::post;
use crate::router::RequestHandler;
use crate::ServerError;

pub fn escape(s: &str) -> String {
    s.replace("\\", "\\\\").replace(" ", "\\s")
}

pub async fn compact_with_context(
    context: &mut Context<'_, '_>,
    val: &JValue,
) -> Result<JValue, CompactionError<context::SurfContextLoader>> {
    compact::<context::SurfContextLoader>(
        val,
        &context::outgoing_context(context),
        &JsonLdOptions {
            base: None,
            compact_arrays: Some(true),
            expand_context: None,
            processing_mode: None,
        },
    )
    .await
}

pub fn create_signature<C: surf::middleware::HttpClient + Debug + Unpin + Send + Sync>(
    data: &str,
    key_object: &StoreItem,
    req: surf::Request<C>,
) -> Result<surf::Request<C>, Box<dyn std::error::Error + Send + Sync>> {
    let digest = Sha256::digest_str(data);
    let digest = base64::encode_config(&digest, base64::STANDARD);

    let mut req = req.set_header("digest", format!("SHA-256={}", digest));

    let private_key = if let [Pointer::Value(Value {
        value: JValue::String(strval),
        ..
    })] = &key_object.sub(kroeg!(meta)).unwrap()[sec!(privateKeyPem)] as &[_]
    {
        PKey::from_rsa(Rsa::private_key_from_pem(strval.as_bytes())?)?
    } else {
        return Ok(req);
    };

    let mut signer = Signer::new(MessageDigest::sha256(), &private_key)?;

    let mut signed = String::new();
    let headers = &["(request-target)", "host", "digest"];
    for val in headers {
        let value = match *val {
            "(request-target)" => format!(
                "(request-target): {} {}{}",
                req.method().as_str().to_lowercase(),
                req.url().path(),
                match req.url().query() {
                    None => format!(""),
                    Some(val) => format!("?{}", val),
                }
            ),

            "host" => format!("{}: {}", val, req.url().host().unwrap()),
            val => format!("{}: {}", val, req.headers().get(val).unwrap_or("")),
        };

        if signed.len() > 0 {
            signed += "\n";
        }

        signed += &value;
    }

    signer.update(signed.as_bytes())?;
    let signature = base64::encode_config(&signer.sign_to_vec()?, base64::STANDARD);

    Ok(req.set_header(
        "Signature",
        format!(
            "keyId=\"{}\",algorithm=\"rsa-sha256\",headers=\"{}\",signature=\"{}\"",
            key_object.id(),
            headers.join(" "),
            signature
        ),
    ))
}

pub async fn deliver_one(
    context: &mut Context<'_, '_>,
    item: &QueueItem,
) -> Result<(), ServerError> {
    match &item.event as &str {
        "deliver" => {
            // bad. urlencode instead.
            let mut data: Vec<_> = item
                .data
                .split(' ')
                .map(|f| f.replace("\\s", " ").replace("\\\\", "\\"))
                .collect();
            let inbox = data.remove(1);
            let itemid = data.remove(0);

            println!(" + preparing to deliver {:?} to {:?}", itemid, inbox);

            let item = match context
                .entity_store
                .get(itemid, false)
                .await
                .map_err(ServerError::StoreError)?
            {
                Some(sdata) => sdata,
                None => return Ok(()),
            };

            if let [Pointer::Id(id)] = &item.main()[as2!(actor)] as &[_] {
                context.user.subject = id.to_owned();
            } else {
                return Ok(());
            }

            let is_local = match context
                .entity_store
                .get(inbox.to_owned(), true)
                .await
                .map_err(ServerError::StoreError)?
            {
                Some(val) => val.is_owned(&context),
                None => false,
            };

            let object = if is_local {
                json!({ "@id": item.id(), "@type": [kroeg!(DeliveryObject)] })
            } else {
                let assembled = assemble(
                    &item,
                    0,
                    context,
                    &LocalOnlyAuthorizer::new(DefaultAuthorizer),
                    &mut HashSet::new(),
                )
                .await
                .map_err(ServerError::HandlerError)?;

                compact_with_context(context, &assembled)
                    .await
                    .map_err(ServerError::CompactionError)?
            };

            let actor = context.user.subject.clone();

            if is_local {
                let handler = post::PostHandler;
                let req = http::Request::builder()
                    .uri(&inbox)
                    .method("POST")
                    .body(Body::from(object.to_string()))
                    .unwrap();
                let response = handler.run(context, req).await?;

                println!(
                    " + deliver {} to {}: {}",
                    item.id(),
                    inbox,
                    response.status()
                );
            } else {
                let mut request = surf::post(&inbox);

                let owner = match context
                    .entity_store
                    .get(actor, false)
                    .await
                    .map_err(ServerError::StoreError)?
                {
                    Some(owner) => owner,
                    None => return Ok(()),
                };

                let blob = object.to_string();

                if let [Pointer::Id(key_id)] = &owner.main()[sec!(publicKey)] as &[_] {
                    if let Some(key) = context
                        .entity_store
                        .get(key_id.to_owned(), true)
                        .await
                        .map_err(ServerError::StoreError)?
                    {
                        request = create_signature(&blob, &key, request)
                            .map_err(ServerError::HandlerError)?;
                    }
                }

                let response = request
                    .body_string(blob)
                    .set_header(
                        "Content-Type",
                        "application/ld+json; profile=\"https://www.w3.org/ns/activitystreams\"",
                    )
                    .timeout(Duration::from_secs(7))
                    .await
                    .map_err(ServerError::HttpError)?;

                println!(
                    " + deliver {} to {}: {}",
                    item.id(),
                    inbox,
                    response.status()
                );
            }

            Ok(())
        }

        _ => Err(ServerError::HandlerError("no idea how to handle".into())),
    }
}

pub async fn register_delivery(
    context: &mut Context<'_, '_>,
    item: String,
    towards: String,
) -> Result<(), StoreError> {
    context
        .queue_store
        .add(
            "deliver".to_owned(),
            format!("{} {}", escape(&item), escape(&towards)),
        )
        .await
}

use std::panic::AssertUnwindSafe;

pub async fn loop_deliver(context: &mut Context<'_, '_>) -> Result<(), ServerError> {
    println!("+ Delivery thread start");
    loop {
        let item = context
            .queue_store
            .get_item()
            .await
            .map_err(ServerError::StoreError)?;
        match item {
            Some(val) => {
                let delivery = AssertUnwindSafe(deliver_one(context, &val))
                    .catch_unwind()
                    .await;

                match delivery {
                    Ok(Ok(())) => {
                        context
                            .queue_store
                            .mark_success(val)
                            .await
                            .map_err(ServerError::StoreError)?;
                    }

                    Ok(Err(e)) => {
                        println!("+ queue handling fail: {}", e);
                        context
                            .queue_store
                            .mark_failure(val)
                            .await
                            .map_err(ServerError::StoreError)?;
                    }

                    Err(e) => {
                        println!("+ queue handling fail: {:?} (panic)", e);
                        context
                            .queue_store
                            .mark_failure(val)
                            .await
                            .map_err(ServerError::StoreError)?;
                    }
                }
            }

            None => {
                Delay::new(Duration::from_secs(10)).await.unwrap();
            }
        };
    }
}
