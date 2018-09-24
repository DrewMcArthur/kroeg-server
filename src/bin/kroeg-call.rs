#![feature(generators, use_extern_macros)]

extern crate base64;
extern crate chashmap;
extern crate diesel;
extern crate dotenv;
extern crate futures_await as futures;
extern crate hyper;
extern crate jsonld;
extern crate kroeg_cellar;
extern crate kroeg_tap_activitypub;
extern crate openssl;
extern crate toml;

#[macro_use]
extern crate kroeg_tap;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate serde_json;

#[path = "../config.rs"]
mod config;

use diesel::prelude::*;
use futures::prelude::{await, *};
use jsonld::nodemap::{Pointer, Value};
use kroeg_cellar::QuadClient;
use kroeg_tap::{untangle, Context, EntityStore, MessageHandler, User};
use kroeg_tap_activitypub::handlers::CreateActorHandler;
use openssl::{hash::MessageDigest, pkey::PKey, rsa::Rsa, sign::Signer};
use serde_json::Value as JValue;
use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::fs::File;
use std::io::Read;

fn read_config() -> config::Config {
    let config_url = dotenv::var("CONFIG").unwrap_or("server.toml".to_owned());
    let mut file = File::open(&config_url).expect("Server config file not found!");
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).expect("Failed to read file!");

    toml::from_slice(&buffer).expect("Invalid config file!")
}

#[async]
fn create_auth<T: EntityStore + 'static>(store: T, id: String) -> Result<(), T::Error> {
    let person = await!(store.get(id, false))?.unwrap();
    let mut key = None;
    for val in &person.main()[sec!(publicKey)] {
        if let Pointer::Id(id) = val {
            key = Some(id.to_owned());
            break;
        }
    }

    if key == None {
        eprintln!("Cannot create authentication for user: no key");
        return Ok(());
    }

    let keyid = key.unwrap();
    let mut key = await!(store.get(keyid.to_owned(), false))?.unwrap();
    let private = &key.meta()[sec!(privateKeyPem)];
    let key = if private.len() != 1 {
        eprintln!("Cannot create authentication for user: no private key");
        return Ok(());
    } else {
        if let Pointer::Value(Value {
            value: JValue::String(strval),
            ..
        }) = private[0].clone()
        {
            PKey::from_rsa(Rsa::private_key_from_pem(strval.as_bytes()).unwrap()).unwrap()
        } else {
            eprintln!("Invalid value for privateKeyPem");
            return Ok(());
        }
    };

    let mut signer = Signer::new(MessageDigest::sha256(), &key).unwrap();
    let signed = format!(
        "{}.{}",
        base64::encode_config(
            json!({
            "typ": "JWT",
            "alg": "RS256",
            "kid": keyid
        }).to_string()
            .as_bytes(),
            base64::URL_SAFE_NO_PAD
        ),
        base64::encode_config(
            json!({
            "iss": "kroeg-call",
            "sub": person.id(),
            "exp": 0xFFFFFFFFu32
        }).to_string()
            .as_bytes(),
            base64::URL_SAFE_NO_PAD
        ),
    );

    signer.update(signed.as_bytes()).unwrap();
    let signature = base64::encode_config(&signer.sign_to_vec().unwrap(), base64::URL_SAFE_NO_PAD);

    println!("Bearer {}.{}", signed, signature);

    Ok(())
}

#[async]
fn create_user<T: EntityStore + 'static>(
    mut store: T,
    id: String,
    name: String,
    username: String,
) -> Result<(), Box<Error + Send + Sync + 'static>> {
    let user = json!(
        {
            "@id": id.to_owned(),
            "@type": [as2!(Person)],
            as2!(name): [{"@value": name}],
            as2!(preferredUsername): [{"@value": username}],
        }
    );

    let mut untangled = untangle(user).unwrap();
    for (key, value) in untangled {
        println!("Storing {:?} {:?}", key, value);
        await!(store.put(key, value)).map_err(Box::new)?;
    }

    let config = read_config();
    let context = Context {
        server_base: config.server.base_uri,
        instance_id: config.server.instance_id,

        user: User {
            claims: HashMap::new(),
            issuer: None,
            subject: "anonymous".to_string(),
            audience: Vec::new(),
            token_identifier: "cli".to_string(),
        },
    };
    let actor_handler = CreateActorHandler;

    await!(actor_handler.handle(context, store, "".to_string(), id.to_owned()))?;

    Ok(())
}

fn main() {
    dotenv::dotenv().ok();

    let mut args: Vec<_> = env::args().collect();

    let config = read_config();
    let db = PgConnection::establish(&config.database)
        .expect(&format!("Error connecting to {}", config.database));
    let store = QuadClient::new(db);

    if args.len() < 2 {
        eprintln!("Usage: {} [create / auth]", args[0]);
        return;
    }

    let promise: Box<Future<Item = (), Error = ()> + Send> = match &args[1].to_string() as &str {
        "create" => {
            if args.len() < 5 {
                eprintln!("Usage: {} create id username \"name\"", args[0]);
                return;
            }

            let name = args.remove(4);
            let username = args.remove(3);
            let id = args.remove(2);

            Box::new(create_user(store, id, name, username).map_err(|e| eprintln!("Error: {}", e)))
        }

        "auth" => {
            let id = args.remove(2);
            Box::new(create_auth(store, id).map_err(|e| eprintln!("Error: {}", e)))
        }

        val => {
            eprintln!("Unknown call {}", val);
            return;
        }
    };

    hyper::rt::run(promise);
}
