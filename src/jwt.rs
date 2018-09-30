use base64;
use jsonld::nodemap::Pointer;
use kroeg_tap::{EntityStore, User};
use openssl::{hash::MessageDigest, pkey::PKey, rsa::Rsa, sign::Verifier};
use serde_json::from_slice;
use serde_json::Value as JValue;
use std::collections::HashMap;

use futures::prelude::{await, *};

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

#[async]
pub fn verify<R: EntityStore>(store: R, token: String) -> Result<(R, Option<User>), (R::Error, R)> {
    let mut spl: Vec<_> = token.split('.').map(str::to_owned).collect();
    if spl.len() != 3 {
        return Ok((store, None));
    }

    let header = match base64::decode_config(spl[0].as_bytes(), base64::URL_SAFE_NO_PAD) {
        Ok(header) => header,
        Err(_) => return Ok((store, None)),
    };

    let contents = match base64::decode_config(spl[1].as_bytes(), base64::URL_SAFE_NO_PAD) {
        Ok(contents) => contents,
        Err(_) => return Ok((store, None)),
    };

    let signature = match base64::decode_config(spl[2].as_bytes(), base64::URL_SAFE_NO_PAD) {
        Ok(signature) => signature,
        Err(_) => return Ok((store, None)),
    };

    let headerdata: JWTHeader = match from_slice(&header) {
        Ok(header) => header,
        Err(_) => return Ok((store, None)),
    };

    if headerdata.typ != "JWT" || headerdata.alg != "RS256" {
        return Ok((store, None));
    }

    let contentdata: JWTContents = match from_slice(&contents) {
        Ok(contents) => contents,
        Err(_) => return Ok((store, None)),
    };

    let (key_data, store) = await!(store.get(headerdata.kid.to_owned(), false))?;;
    let key_data = key_data.and_then(|f| f.main()[sec!(publicKeyPem)].iter().next().cloned())
        .and_then(|f| match f {
            Pointer::Value(val) => {
                if let JValue::String(strval) = &val.value {
                    Some(strval.to_owned())
                } else {
                    None
                }
            }

            _ => None,
        }).and_then(|f| Rsa::public_key_from_pem(f.as_bytes()).ok())
        .map(|f| PKey::from_rsa(f).unwrap())
        .unwrap();
    let mut verifier = Verifier::new(MessageDigest::sha256(), &key_data).unwrap();

    let mut to_sign = Vec::new();
    to_sign.append(&mut spl[0].as_bytes().iter().map(|f| *f).collect());
    to_sign.push(b'.');
    to_sign.append(&mut spl[1].as_bytes().iter().map(|f| *f).collect());

    verifier.update(&to_sign).unwrap();
    if let Ok(true) = verifier.verify(&signature) {
        Ok((
            store,
            Some(User {
                claims: contentdata.other,
                issuer: Some(contentdata.iss),
                subject: contentdata.sub,
                audience: contentdata.aud.unwrap_or(vec![]),
                token_identifier: contentdata.jti.unwrap_or("".to_owned()),
            }),
        ))
    } else {
        Ok((store, None))
    }
}
