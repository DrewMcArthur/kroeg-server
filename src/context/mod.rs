use futures::prelude::*;

use jsonld::{compact as jsonld_compact, error::CompactionError, JsonLdOptions};
use kroeg_tap::Context;
use serde_json::{from_slice, Value};

mod loader;
pub use self::loader::*;

/// Gets the context that is applied after the AS2 context.
pub fn read_context() -> Value {
    from_slice(include_bytes!("context.json")).unwrap()
}

/// The supplement is used to override the @context on incoming documents.
pub fn read_supplement() -> Value {
    from_slice(include_bytes!("extra_context.json")).unwrap()
}

/// Apply the supplement to a JSON document, to ensure that it can be more efficiently handled
/// by Kroeg.
pub fn apply_supplement(val: Value) -> Value {
    match val {
        Value::Object(mut obj) => {
            let removed = obj.remove("@context");
            match removed {
                Some(Value::Array(mut arr)) => {
                    arr.push(read_supplement());
                    obj.insert("@context".to_owned(), Value::Array(arr));
                }
                Some(other) => {
                    obj.insert(
                        "@context".to_owned(),
                        Value::Array(vec![other, read_supplement()]),
                    );
                }
                _ => {}
            };
            Value::Object(obj)
        }

        val => val,
    }
}

/// Returns the outgoing context as used in @context, for all outgoing documents.
pub fn outgoing_context(context: &Context) -> Value {
    Value::Array(vec![
        Value::String("https://www.w3.org/ns/activitystreams".to_owned()),
        Value::String(format!("{}/-/context", context.server_base)),
    ])
}

pub fn compact(
    context: &Context,
    value: Value,
) -> impl Future<Item = Value, Error = CompactionError<HyperContextLoader>> + Send {
    jsonld_compact::<HyperContextLoader>(
        value,
        outgoing_context(context),
        JsonLdOptions {
            base: None,
            compact_arrays: Some(true),
            expand_context: None,
            processing_mode: None,
        },
    )
}
