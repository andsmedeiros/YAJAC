pub mod adapter;
pub mod http_wrappers;
pub mod resourceful;
pub mod spec;

#[macro_export]
macro_rules! extract {
    ($obj:expr, [$($field:ident),+]) => {{
        use std::collections::HashMap;
        use serde_json::{Value, json};
        let entries = vec![ $(
            (stringify!($field).to_string(), json!(&$obj.$field)),
        )+ ];
        Some(HashMap::from_iter(entries.into_iter()))
    }};
}