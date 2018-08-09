use futures::prelude::{await, *};
use jsonld::nodemap::{Entity, Pointer, Value};
use kroeg_tap::{EntityStore, StoreItem};
use serde_json::Value as JValue;

use super::structs;
fn best_string(data: &Vec<Pointer>) -> Option<String> {
    for val in data {
        if let Pointer::Value(Value {
            value: JValue::String(strval),
            ..
        }) = val
        {
            return Some(strval.to_owned());
        }
    }

    None
}
fn best_id(data: &Vec<Pointer>) -> Option<String> {
    for val in data {
        if let Pointer::Id(strval) = val {
            return Some(strval.to_owned());
        }
    }

    None
}

fn encode_id(val: String) -> String {
    val
}

fn decode_id(val: String) -> String {
    val
}

#[async]
pub fn account<T: EntityStore>(
    store: T,
    id: String,
) -> Result<(T, Option<structs::Account>), T::Error> {
    println!("Reading {:?} {:?}", store, id);
    let elem = match await!(store.get(id, true))? {
        Some(val) => val,
        None => return Ok((store, None)),
    };

    let username =
        best_string(&elem.main()[as2!(preferredUsername)]).unwrap_or_else(|| elem.id().to_owned());
    let acct = username.to_owned(); // todo: better
    let display_name = best_string(&elem.main()[as2!(name)]).unwrap_or_else(|| username.to_owned());
    let created_at = best_string(&elem.main()[as2!(published)])
        .unwrap_or_else(|| "2000-01-01T00:00:00".to_owned());
    let note = best_string(&elem.main()[as2!(summary)]).unwrap_or_else(|| "".to_owned());

    Ok((
        store,
        Some(structs::Account {
            id: encode_id(elem.id().to_owned()),
            username,
            acct,
            display_name,
            created_at,
            note,

            locked: false,
            followers_count: 0,
            following_count: 0,
            statuses_count: 0,
            url: elem.id().to_owned(),
            avatar: "https://placehold.it/52".to_owned(),
            avatar_static: "https://placehold.it/52".to_owned(),
            header: "https://placehold.it/52".to_owned(),
            header_static: "https://placehold.it/52".to_owned(),
            moved: None,
            fields: Vec::new(),
            bot: Some(false),
        }),
    ))
}