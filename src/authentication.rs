//! Simple, insecure, authentication method to mock the server for now.

use super::config;
use jsonld::nodemap::Pointer;
use kroeg_tap::{EntityStore, User};

use base64::decode;
use openssl::{hash::MessageDigest, pkey::PKey, rsa::Rsa, sign::Verifier};
use serde_json::Value as JValue;

use futures::prelude::{await, *};
use std::collections::HashMap;

use http::request::Parts;
use jwt::verify;
use std::io::Write;

pub fn build_header_magic(parts: &Parts, sig: Vec<String>) -> Vec<u8> {
    let mut result = Vec::new();

    let mut is_first = true;
    for value in sig {
        if !is_first {
            let _ = write!(result, "\n");
        }

        if value == "(request-target)" {
            let _ = write!(
                result,
                "(request-target): {} {}",
                parts.method.as_str().to_lowercase(),
                parts.uri.path()
            );
            if let Some(query) = parts.uri.query() {
                let _ = write!(result, "?{}", query);
            }
        } else if value != "" {
            let _ = write!(result, "{}: ", value);
            let mut is_first = true;
            for header in parts.headers.get_all(value) {
                if !is_first {
                    let _ = write!(result, ", ");
                }

                result.append(&mut header.as_bytes().iter().map(|f| *f).collect());

                is_first = false;
            }
        }

        is_first = false;
    }

    result
}

#[async]
pub fn verify_http_signature<R: EntityStore>(
    req: Parts,
    store: R,
) -> Result<(Parts, R, Option<User>), R::Error> {
    if let Some(val) = req
        .headers
        .get("Signature")
        .and_then(|f| f.to_str().ok().map(str::to_string))
    {
        let values: Vec<_> = val.split(',').map(str::to_string).collect();
        let mut map = HashMap::new();
        for value in values {
            let (name, value) = value.split_at(value.find('=').unwrap() + 1);
            let name = name.trim_matches('=').to_owned();
            let value = value.trim_matches('"').to_owned();
            map.insert(name, value);
        }

        match (
            map.get("keyId").cloned(),
            map.get("algorithm").cloned(),
            map.get("headers").cloned(),
            map.get("signature").cloned(),
        ) {
            (Some(key_id), Some(algorithm), Some(headers), Some(signature)) => {
                let mut key_id = key_id.to_owned();
                if key_id.starts_with("acct:") {
                    // XXX Mastodon hack, clean this code up later
                    let spl = key_id.split(':').collect::<Vec<_>>()[1]
                        .split('@')
                        .map(String::from)
                        .collect::<Vec<_>>();
                    key_id = format!("https://{}/users/{}#public-key", spl[1], spl[0]);
                }
                let key_data = await!(store.get(key_id.to_owned(), false))?;
                if let Some(key_data) = key_data {
                    let pem_data = key_data.main()[sec!(publicKeyPem)]
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
                        }).and_then(|f| Rsa::public_key_from_pem(f.as_bytes()).ok());

                    if let Some(key) = pem_data {
                        let key = PKey::from_rsa(key).unwrap();
                        let signature = decode(signature.as_bytes()).unwrap(); // i know, bad
                        let owner = match key_data.main()[sec!(owner)].iter().next() {
                            Some(Pointer::Id(id)) => Some(id.to_owned()),
                            _ => None,
                        }.unwrap();

                        match &*algorithm as &str {
                            "rsa-sha256" => {
                                let mut verifier =
                                    Verifier::new(MessageDigest::sha256(), &key).unwrap();
                                let header_magic = build_header_magic(
                                    &req,
                                    headers.split(' ').map(str::to_string).collect(),
                                );
                                verifier.update(&header_magic).unwrap();
                                let result = verifier.verify(&signature).unwrap();
                                if result {
                                    return Ok((
                                        req,
                                        store,
                                        Some(User {
                                            claims: HashMap::new(),
                                            issuer: Some(key_id.to_owned()),
                                            subject: owner,
                                            audience: vec![],
                                            token_identifier: "http-signature".to_owned(),
                                        }),
                                    ));
                                }
                            }

                            _ => { /* */ }
                        }
                    }
                }
            }

            (_, _, _, _) => { /* */ }
        };
    }

    Ok((req, store, None))
}

pub fn anonymous() -> User {
    User {
        claims: HashMap::new(),
        issuer: None,
        subject: "anonymous".to_owned(),
        audience: vec![],
        token_identifier: "anon".to_owned(),
    }
}

#[async]
pub fn user_from_request<R: EntityStore>(
    config: config::Config,
    req: Parts,
    mut store: R,
) -> Result<(config::Config, Parts, R, User), R::Error> {
    if let Some(val) = req
        .headers
        .get("Authorization")
        .and_then(|f| f.to_str().ok().map(str::to_string))
    {
        let bearer: Vec<_> = val.split(' ').map(str::to_string).collect();
        if bearer[0] == "Bearer" {
            let (nstore, user) = await!(verify(store, bearer[1].to_owned()))?;
            store = nstore;
            if let Some(user) = user {
                return Ok((config, req, store, user));
            }
        }
    }

    await!(verify_http_signature(req, store).map(|(req, store, data)| (
        req,
        store,
        match data {
            Some(data) => data,
            None => anonymous(),
        },
    ))).map(move |(req, store, user)| (config, req, store, user))
}
