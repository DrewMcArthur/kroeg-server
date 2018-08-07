#![feature(generators, use_extern_macros)]

extern crate toml;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate lazy_static;

extern crate chashmap;
extern crate futures_await as futures;
extern crate hyper;
extern crate jsonld;
#[macro_use]
extern crate serde_json;

#[path = "../config.rs"]
mod config;

extern crate diesel;
extern crate dotenv;
extern crate kroeg_cellar;
#[macro_use]
extern crate kroeg_tap;

use diesel::prelude::*;
use std::env;

use jsonld::nodemap::Pointer;

use kroeg_cellar::QuadClient;
use kroeg_tap::EntityStore;

use kroeg_tap::untangle;
use std::fs::File;
use std::io::Read;

use futures::future;
use futures::prelude::{await, *};

fn read_config() -> config::Config {
    let config_url = dotenv::var("CONFIG").unwrap_or("server.toml".to_owned());
    let mut file = File::open(&config_url).expect("Server config file not found!");
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).expect("Failed to read file!");

    toml::from_slice(&buffer).expect("Invalid config file!")
}

#[async]
fn create_user<T: EntityStore + 'static>(
    mut store: T,
    id: String,
    name: String,
    username: String,
) -> Result<(), T::Error> {
    let inbox_id = format!("{}/inbox", id);
    let outbox_id = format!("{}/outbox", id);

    let user = json!(
        {
            "@id": id.to_owned(),
            "@type": [as2!(Person)],
            as2!(name): [{"@value": name}],
            as2!(preferredUsername): [{"@value": username}],
            as2!(outbox): [{"@id": outbox_id, "@type": [as2!(OrderedCollection)], as2!(attributedTo): [{"@id": id.to_owned()}]}],
            ldp!(inbox): [{"@id": inbox_id, "@type": [as2!(OrderedCollection)], as2!(attributedTo): [{"@id": id.to_owned()}]}]
        }
    );

    let mut untangled = untangle(user).unwrap();
    untangled
        .get_mut(&inbox_id)
        .unwrap()
        .meta()
        .get_mut(kroeg!(box))
        .push(Pointer::Id(ldp!(inbox).to_owned()));
    untangled
        .get_mut(&outbox_id)
        .unwrap()
        .meta()
        .get_mut(kroeg!(box))
        .push(Pointer::Id(as2!(outbox).to_owned()));
    for (key, value) in untangled {
        await!(store.put(key, value))?;
    }
    Ok(())
}

fn main() {
    dotenv::dotenv().ok();

    let mut args: Vec<_> = env::args().collect();

    if args.len() != 4 {
        eprintln!("Usage: {} <id> <username> <name>", args[0]);
    }

    let name = args.remove(3);
    let username = args.remove(2);
    let id = args.remove(1);

    let config = read_config();
    let db = PgConnection::establish(&config.database)
        .expect(&format!("Error connecting to {}", config.database));
    let mut store = QuadClient::new(db);

    hyper::rt::run(create_user(store, id, name, username).map_err(|e| eprintln!("Error: {}", e)));
}
