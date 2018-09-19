use futures::prelude::{await, *};
use jsonld::nodemap::{Entity, Pointer, Value};
use kroeg_tap::{EntityStore, StoreItem};
use serde_json::Value as JValue;
use serde::Serialize;
use base64;

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

fn best_bool(data: &Vec<Pointer>) -> Option<bool> {
    for val in data {
        if let Pointer::Value(Value {
            value: JValue::Bool(val),
            ..
        }) = val
        {
            return Some(*val);
        }
    }

    None
}

pub fn encode_id(val: String) -> String {
    base64::encode_config(&val, base64::URL_SAFE)
}

pub fn decode_id(val: String) -> String {
    base64::decode_config(&val, base64::URL_SAFE).ok().and_then(|v| String::from_utf8(v).ok()).unwrap_or_else(|| "".to_string())
}

#[async]
pub fn account<T: EntityStore>(
    store: T,
    id: String,
) -> Result<(T, Option<structs::Account>), T::Error> {
    println!("Translating account {:?}", id);
    let elem = match await!(store.get(id, false))? {
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

#[async]
pub fn timeline<T: EntityStore, R: Serialize, U: Future<Item=(T, Option<R>), Error = T::Error> + 'static + Send, V: Fn(T, String) -> U + 'static>(
    mut store: T,
    collection: String,
    query: Option<String>,
    call: V,
) -> Result<(T, Vec<R>, Option<String>, Option<String>), T::Error> {
    let data = await!(store.read_collection(collection, None, query))?;

    let mut result = Vec::new();
    for item in data.items {
        let (s, val) = await!(call(store, item))?;
        store = s;
        if let Some(val) = val {
            result.push(val);
        }
    }

    Ok((store, result, data.before, data.after))
}

#[async(boxed_send)]
pub fn reblog_status<T: EntityStore>(
    store: T,
    elem: StoreItem
) -> Result<(T, Option<structs::Status>), T::Error> {

    let object = match best_id(&elem.main()[as2!(object)]) {
        Some(val) => val,
        None => return Ok((store, None))
    };
    println!("Translating announce-status {:?} -> {:?}", elem.main().id, object);

    let (store, status) = await!(status(store, object))?;
    let status = match status {
        Some(val) => val,
        None => return Ok((store, None))
    };

    let (store, account) = await!(account(store, best_id(&elem.main()[as2!(actor)]).unwrap()))?;
    let account = match account {
        Some(val) => val,
        None => return Ok((store, None))
    };

    let mut newstatus = status.clone();
    newstatus.account = account;
    newstatus.reblog = Some(Box::new(status));

    Ok((store, Some(newstatus)))
}

#[async]
pub fn status<T: EntityStore>(
    store: T,
    id: String,
) -> Result<(T, Option<structs::Status>), T::Error> {
    let mut elem = match await!(store.get(id, false))? {
        Some(val) => val,
        None => return Ok((store, None)),
    };

    println!("{:?}", elem.main().types);

    if elem.main().types.contains(&String::from(as2!(Announce))) {
        return await!(reblog_status(store, elem));
    }

    if elem.main().types.contains(&String::from(as2!(Create))) {
        let object = match best_id(&elem.main()[as2!(object)]) {
            Some(val) => val,
            None => return Ok((store, None))
        };

        elem = match await!(store.get(object, false))? {
            Some(val) => val,
            None => return Ok((store, None))
        };
    }

    if !elem.main().types.contains(&String::from(as2!(Note))) {
        return Ok((store, None));
    }

    println!("Translating status {:?}", elem.main().id);

    let url = best_id(&elem.main()[as2!(url)]);
    let (store, account) = await!(account(store, best_id(&elem.main()[as2!(attributedTo)]).unwrap()))?;
    let account = match account {
        Some(val) => val,
        None => return Ok((store, None))
    };

    let in_reply_to_id = best_id(&elem.main()[as2!(inReplyTo)]).map(|f| encode_id(f));
    let content = best_string(&elem.main()[as2!(content)]).unwrap_or_else(|| "".to_owned());
    let created_at = best_string(&elem.main()[as2!(published)])
        .unwrap_or_else(|| "2000-01-01T00:00:00".to_owned());
    let sensitive = best_bool(&elem.main()[as2!(sensitive)]).unwrap_or(false);
    let spoiler_text = best_string(&elem.main()[as2!(summary)]).unwrap_or_else(|| "".to_owned());


    Ok((
        store,
        Some(structs::Status {
            id: encode_id(elem.id().to_owned()),
            uri: elem.id().to_owned(),
            url: url,
            account: account,
            in_reply_to_id: in_reply_to_id,
            in_reply_to_account_id: None,
            reblog: None,
            content: content,
            created_at: created_at,
            emojis: Vec::new(),
            reblogs_count: 0,
            favourites_count: 0,
            reblogged: Some(false),
            favourited: Some(false),
            muted: Some(false),
            sensitive: sensitive,
            spoiler_text: spoiler_text,
            visibility: "public".to_owned(), // xxx bad
            media_attachments: Vec::new(),
            mentions: Vec::new(),
            tags: Vec::new(),
            application: None,
            language: None,
            pinned: Some(false),
        })
    ))
}
