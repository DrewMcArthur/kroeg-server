use base64;
use jsonld::nodemap::{Pointer, Value};
use kroeg_tap::{sec, EntityStore, StoreError, User};
use openssl::{hash::MessageDigest, pkey::PKey, rsa::Rsa, sign::Verifier};
use serde::{Deserialize, Serialize};
use serde_json::{from_slice, Value as JValue};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Debug)]
struct JWTHeader {
    pub typ: String,
    pub alg: String,
    pub kid: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct JWTContents {
    pub iss: String,
    pub sub: String,
    pub aud: Option<Vec<String>>,
    pub exp: u32,
    pub nbf: Option<u32>,
    pub iat: Option<u32>,
    pub jti: Option<String>,
    #[serde(flatten)]
    pub other: HashMap<String, String>,
}

pub async fn verify(
    store: &mut dyn EntityStore,
    token: String,
) -> Result<Option<User>, StoreError> {
    let mut spl: Vec<_> = token.split('.').map(str::to_owned).collect();
    if spl.len() != 3 {
        return Ok(None);
    }

    let header = match base64::decode_config(spl[0].as_bytes(), base64::URL_SAFE_NO_PAD) {
        Ok(header) => header,
        Err(_) => return Ok(None),
    };

    let contents = match base64::decode_config(spl[1].as_bytes(), base64::URL_SAFE_NO_PAD) {
        Ok(contents) => contents,
        Err(_) => return Ok(None),
    };

    let signature = match base64::decode_config(spl[2].as_bytes(), base64::URL_SAFE_NO_PAD) {
        Ok(signature) => signature,
        Err(_) => return Ok(None),
    };

    let headerdata: JWTHeader = match from_slice(&header) {
        Ok(header) => header,
        Err(_) => return Ok(None),
    };

    if headerdata.typ != "JWT" || headerdata.alg != "RS256" {
        return Ok(None);
    }

    let contentdata: JWTContents = match from_slice(&contents) {
        Ok(contents) => contents,
        Err(_) => return Ok(None),
    };

    let key_data = match store.get(headerdata.kid.to_owned(), false).await? {
        Some(some) => some,
        None => return Ok(None),
    };

    let key = if let [Pointer::Value(Value {
        value: JValue::String(key),
        ..
    })] = &key_data.main()[sec!(publicKeyPem)] as &[Pointer]
    {
        let key = Rsa::public_key_from_pem(key.as_bytes())?;

        PKey::from_rsa(key)?
    } else {
        return Ok(None);
    };

    let mut verifier = Verifier::new(MessageDigest::sha256(), &key).unwrap();

    let mut to_sign = Vec::new();
    to_sign.append(&mut spl[0].as_bytes().iter().map(|f| *f).collect());
    to_sign.push(b'.');
    to_sign.append(&mut spl[1].as_bytes().iter().map(|f| *f).collect());

    verifier.update(&to_sign).unwrap();

    if verifier.verify(&signature)? {
        Ok(Some(User {
            claims: contentdata.other,
            issuer: Some(contentdata.iss),
            subject: contentdata.sub,
            audience: contentdata.aud.unwrap_or(vec![]),
            token_identifier: contentdata.jti.unwrap_or("".to_owned()),
        }))
    } else {
        Ok(None)
    }
}
