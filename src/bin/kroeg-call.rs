#[path = "../config.rs"]
mod config;

use jsonld::nodemap::{Pointer, Value};
use kroeg_cellar::{CellarConnection, CellarEntityStore};
use kroeg_tap::{
    as2, kroeg, sec, untangle, Context, EntityStore, MessageHandler, StoreError, User,
};
use kroeg_tap_activitypub::handlers::CreateActorHandler;
use openssl::{hash::MessageDigest, pkey::PKey, rsa::Rsa, sign::Signer};
use serde_json::{json, Value as JValue};
use std::collections::HashMap;
use std::env;

async fn create_auth(store: &mut dyn EntityStore, id: String) -> Result<(), StoreError> {
    let person = store.get(id, false).await?.unwrap();

    let keyid = if let [Pointer::Id(id)] = &person.main()[sec!(publicKey)] as &[_] {
        id.to_owned()
    } else {
        eprintln!("Cannot create authentication for user: no key");
        return Ok(());
    };

    let mut key = store.get(keyid.to_owned(), false).await?.unwrap();
    let private = if let [Pointer::Value(Value {
        value: JValue::String(strval),
        ..
    })] = &key.meta()[sec!(privateKeyPem)] as &[_]
    {
        PKey::from_rsa(Rsa::private_key_from_pem(strval.as_bytes())?)?
    } else {
        eprintln!("Cannot create authentication for user: no private key");
        return Ok(());
    };

    let mut signer = Signer::new(MessageDigest::sha256(), &private).unwrap();
    let signed = format!(
        "{}.{}",
        base64::encode_config(
            json!({
                "typ": "JWT",
                "alg": "RS256",
                "kid": keyid
            })
            .to_string()
            .as_bytes(),
            base64::URL_SAFE_NO_PAD
        ),
        base64::encode_config(
            json!({
                "iss": "kroeg-call",
                "sub": person.id(),
                "exp": 0xFFFFFFFFu32
            })
            .to_string()
            .as_bytes(),
            base64::URL_SAFE_NO_PAD
        ),
    );

    signer.update(signed.as_bytes()).unwrap();
    let signature = base64::encode_config(&signer.sign_to_vec().unwrap(), base64::URL_SAFE_NO_PAD);

    println!("Bearer {}.{}", signed, signature);

    Ok(())
}

async fn make_owned(
    config: config::Config,
    store: &mut dyn EntityStore,
    id: String,
) -> Result<(), StoreError> {
    let mut object = store.get(id, true).await?.unwrap();
    object.meta().get_mut(kroeg!(instance)).clear();

    object
        .meta()
        .get_mut(kroeg!(instance))
        .push(Pointer::Value(Value {
            value: JValue::Number(config.server.instance_id.into()),
            type_id: None,
            language: None,
        }));

    store.put(object.id().to_owned(), &mut object).await?;

    Ok(())
}

async fn create_user(
    store: &CellarConnection,
    config: config::Config,
    id: String,
    name: String,
    username: String,
) -> Result<(), StoreError> {
    let user = json!(
        {
            "@id": id.to_owned(),
            "@type": [as2!(Person)],
            as2!(name): [{"@value": name}],
            as2!(preferredUsername): [{"@value": username}],
        }
    );

    let mut entity_store = CellarEntityStore::new(store);
    let mut queue_store = CellarEntityStore::new(store);

    let untangled = untangle(&user)?;
    for (key, mut value) in untangled {
        println!("Storing {:?} {:?}", key, value);
        entity_store.put(key, &mut value).await?;
    }

    let mut context = Context {
        server_base: config.server.base_uri,
        instance_id: config.server.instance_id,
        entity_store: &mut entity_store,
        queue_store: &mut queue_store,

        user: User {
            claims: HashMap::new(),
            issuer: None,
            subject: "anonymous".to_string(),
            audience: Vec::new(),
            token_identifier: "cli".to_string(),
        },
    };

    let actor_handler = CreateActorHandler;
    actor_handler
        .handle(&mut context, &mut "".to_string(), &mut id.to_owned())
        .await?;

    Ok(())
}

async fn do_code() -> Result<(), StoreError> {
    dotenv::dotenv().ok();

    let mut args: Vec<_> = env::args().collect();

    let config = config::read_config();
    let db = CellarConnection::connect(
        &config.database.server,
        &config.database.username,
        &config.database.password,
        &config.database.database,
    )
    .await?;

    if args.len() < 2 {
        eprintln!("Usage: {} [create / auth / own]", args[0]);
        return Ok(());
    }

    match &args[1].to_string() as &str {
        "create" => {
            if args.len() < 5 {
                eprintln!("Usage: {} create id username \"name\"", args[0]);
                return Ok(());
            }

            let name = args.remove(4);
            let username = args.remove(3);
            let id = args.remove(2);

            create_user(&db, config, id, name, username).await?;
        }

        "auth" => {
            let id = args.remove(2);
            create_auth(&mut CellarEntityStore::new(&db), id).await?;
        }

        "own" => {
            let id = args.remove(2);
            make_owned(config, &mut CellarEntityStore::new(&db), id).await?;
        }

        val => {
            eprintln!("Unknown call {}", val);
            return Ok(());
        }
    };

    Ok(())
}

fn main() {
    async_std::task::block_on(do_code()).unwrap();
}
