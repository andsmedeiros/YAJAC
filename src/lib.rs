pub mod adapter;
pub mod http_wrappers;
pub mod resourceful;
pub mod spec;

#[macro_export]
macro_rules! field_entry {
    ($obj:expr, $field:ident) => {
        (stringify!($field).to_string(), serde_json::json!(&$obj.$field))
    };
}

#[macro_export]
macro_rules! extract {
    ($obj:expr, [$($field:ident),+]) => {
        [ $( yajac::field_entry!($obj, $field), )+ ]
            .into_iter()
            .collect::<std::collections::HashMap<_, _>>()
            .into()
    };
}

#[macro_export]
macro_rules! extract_filtered {
    ($obj:expr, [$($field:ident),+], $filter:expr) => {
        match $filter {
            None => yajac::extract!($obj, [$($field),+]),
            Some(filter) => filter
                .into_iter()
                .filter_map(|field| match field.as_str() {
                    $(
                        stringify!($field) =>
                            yajac::field_entry!($obj, $field).into(),
                    )+
                    _ => None,
                })
                .collect::<std::collections::HashMap<_, _>>()
                .into()
        }
    };
}