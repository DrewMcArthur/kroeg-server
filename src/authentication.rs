use base64::decode;
use http::request::Parts;
use jsonld::nodemap::{Pointer, Value};
use kroeg_tap::{sec, EntityStore, StoreError, User};
use openssl::{hash::MessageDigest, pkey::PKey, rsa::Rsa, sign::Verifier};
use serde_json::Value as JValue;
use std::collections::HashMap;
use std::io::Write;

use crate::jwt::verify;

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

pub async fn verify_http_signature(
    req: &Parts,
    store: &mut dyn EntityStore,
) -> Result<Option<User>, StoreError> {
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
                let mut key_id_data = key_id.to_owned();
                if key_id.starts_with("acct:") {
                    // XXX Mastodon hack, clean this code up later
                    let spl = key_id.split(':').collect::<Vec<_>>()[1]
                        .split('@')
                        .map(String::from)
                        .collect::<Vec<_>>();
                    key_id_data = format!("https://{}/users/{}#public-key", spl[1], spl[0]);
                }
                let key_data = match store.get(key_id_data, false).await? {
                    Some(key_data) => key_data,
                    None => return Ok(None),
                };

                if let [Pointer::Value(Value {
                    value: JValue::String(key_pem),
                    ..
                })] = &key_data.main()[sec!(publicKeyPem)] as &[Pointer]
                {
                    let key = Rsa::public_key_from_pem(key_pem.as_bytes())?;
                    let key = PKey::from_rsa(key)?;
                    let signature = decode(signature.as_bytes()).unwrap(); // i know, bad
                    let owner =
                        if let [Pointer::Id(id)] = &key_data.main()[sec!(owner)] as &[Pointer] {
                            id.to_owned()
                        } else {
                            return Ok(None);
                        };

                    match &*algorithm as &str {
                        "rsa-sha256" => {
                            let mut verifier = Verifier::new(MessageDigest::sha256(), &key)?;
                            let header_magic = build_header_magic(
                                &req,
                                headers.split(' ').map(str::to_string).collect(),
                            );

                            verifier.update(&header_magic).unwrap();
                            let result = verifier.verify(&signature)?;
                            if result {
                                return Ok(Some(User {
                                    claims: HashMap::new(),
                                    issuer: Some(key_id.to_owned()),
                                    subject: owner,
                                    audience: vec![],
                                    token_identifier: "http-signature".to_owned(),
                                }));
                            }
                        }
                        _ => { /* */ }
                    }
                }
            }

            (_, _, _, _) => { /* */ }
        };
    }

    Ok(None)
}

pub fn anonymous() -> User {
    User {
        claims: HashMap::new(),
        issuer: None,
        subject: "https://example.com/~user".to_owned(),
        audience: vec![],
        token_identifier: "anon".to_owned(),
    }
}

pub async fn user_from_request(
    req: &Parts,
    store: &mut dyn EntityStore,
) -> Result<User, StoreError> {
    if let Some(val) = req
        .headers
        .get("Authorization")
        .and_then(|f| f.to_str().ok())
    {
        let bearer: Vec<_> = val.split(' ').collect();
        if bearer[0] == "Bearer" && bearer.len() == 2 {
            let user = verify(store, bearer[1].to_owned()).await?;
            if let Some(user) = user {
                return Ok(user);
            }
        }
    }

    match verify_http_signature(req, store).await? {
        Some(data) => Ok(data),
        None => Ok(anonymous()),
    }
}
