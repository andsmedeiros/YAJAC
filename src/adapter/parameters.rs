use crate::http_wrappers::Uri;
use regex::Regex;
use std::{
    borrow::Borrow,
    collections::HashMap, sync::LazyLock,
};

mod regex_builder {
    pub(super) static ID: &'static str = r"[a-zA-Z0-9](?:[-_]*[a-zA-Z0-9]+)*";
}

static SORT_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    use regex_builder::ID;
    Regex::new(format!(r"\A(\-?)({ID})\z").as_str()).unwrap()
});

static QUERY_PARAM_FAMILY_REGEX : LazyLock<Regex> = LazyLock::new(|| {
    use regex_builder::ID;
    Regex::new(format!(r"\A({ID})((?:\[{ID}])+)\z").as_str()).unwrap()
});

static QUERY_PARAM_PATH_SEGMENT_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    use regex_builder::ID;
    Regex::new(format!(r"\[({ID})]").as_str()).unwrap()
});

pub type FieldsParameters = HashMap<String, Vec<String>>;
pub type IncludeParameters = Vec<String>;
pub type FilterParameters = HashMap<String, String>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SortDirection { Ascending, Descending }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortingField { field: String, direction: SortDirection }
pub type SortParameters = Vec<SortingField>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parameters {
    pub fields: Option<FieldsParameters>,
    pub include: Option<IncludeParameters>,
    pub filter: Option<FilterParameters>,
    pub sort: Option<SortParameters>
}

impl Default for Parameters {
    fn default() -> Self {
        Parameters {
            fields: None,
            include: None,
            filter: None,
            sort: None
        }
    }
}

impl Parameters {
    pub fn new(uri: &Uri) -> Parameters {
        match uri.query() {
            None => return Parameters::default(),
            Some(query) => Self::parse_query(query)
        }
    }

    pub fn fields_for(&self, kind: impl Borrow<str>) -> Option<&Vec<String>> {
        match self.fields {
            None => None,
            Some(ref fields) => fields.get(kind.borrow()),
        }
    }

    fn parse_include(include: &str) -> IncludeParameters {
        include.split(",").map(Into::into).collect()
    }

    fn parse_sort(sort: &str) -> SortParameters {
        sort.split(',').filter_map(|entry| {
            let result = SORT_REGEX.captures(entry).map(|c| c.extract());
            if let Some((_, [sign, field])) = result {
                Some(SortingField {
                    field: field.to_string(),
                    direction: match sign {
                        "-" => SortDirection::Descending,
                        "" => SortDirection::Ascending,
                        _ => unreachable!()
                    }
                })
            } else { None }
        })
        .collect()
    }

    fn parse_query_parameter_path(path: &str) -> String {
        QUERY_PARAM_PATH_SEGMENT_REGEX
            .captures_iter(path)
            .map(|c| c.extract())
            .map(|(_, [segment])| segment)
            .collect::<Vec<_>>()
            .join(".")
    }
    
    fn parse_query(query: &str) -> Parameters {
        let mut parameters = Parameters::default();
        let mut fields_params = FieldsParameters::new();
        let mut filter_params = FilterParameters::new();

        for entry in query.split("&") {
            match entry.split("=").collect::<Vec<_>>().as_slice() {
                &["include", include] => {
                    parameters.include = Some(Self::parse_include(include));
                },
                &["sort", sort] => {
                    parameters.sort = Some(Self::parse_sort(sort));
                },
                &[key, value] => {
                    let value = value.split(",").map(Into::into).collect();
                    let result = QUERY_PARAM_FAMILY_REGEX.captures(key).map(|c| c.extract());
                    match result {
                        Some((_, ["fields", path])) => {
                            let path = Self::parse_query_parameter_path(path);
                            fields_params.insert(path, value);
                        },
                        Some((_, ["filter", path])) => {
                            // let path = Self::parse_query_parameter_path(path);
                            // filter_params.insert(path, value)
                            todo!("collect filter params")
                        },
                        _ => continue
                    }
                },
                _ => continue
            }
        }

        if !fields_params.is_empty() {
            parameters.fields = Some(fields_params);
        }

        if !filter_params.is_empty() {
            parameters.filter = Some(filter_params);
        }

        parameters
    }
}

impl<U> From<U> for Parameters
where
    U: Borrow<Uri> + Sized
{
    fn from(uri: U) -> Self {
        Parameters::new(uri.borrow())
    }
}