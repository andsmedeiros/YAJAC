//! Defines a data structure suitable for storing JSON:API-relevant query parameters.
//! Also provides functions to extract those parameters from a query string, which is the only
//! supported construction method.

use super::error::Error;
use crate::database::error::Error::{InvalidEncodingFailure, ParseParameterFailure, SchemaValidationFailure};
use crate::http_wrappers::Uri;
use regex::Regex;
use std::{num::NonZeroU32, sync::LazyLock};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::borrow::Cow;
use urlencoding::decode;
use crate::database::adapters::Adapter as AdapterInterface;
use crate::database::attributes::Attribute;
use crate::database::registry::Registry;
use crate::database::schema::{AttributeType, Relationship, TableSchema};
use crate::database::table::Table;

mod regex_builder {
    /// Generic pattern for identifiers -- model names, field names and relationship names
    pub(crate) static ID: &'static str = r"[a-zA-Z](?:[-_]*[a-zA-Z0-9]+)*";
}

/// Matches exactly a single identifier
static ID_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    use regex_builder::ID;
    Regex::new(format!(r"\A{ID}\z").as_str()).unwrap()
});

/// Matches exactly a sort directive: a single identifier with an optional plus or minus sign,
/// indicating sort direction
static SORT_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    use regex_builder::ID;
    Regex::new(format!(r"\A([-+]?)({ID})\z").as_str()).unwrap()
});

/// Matches exactly a filter directive: a supported operand and a filter term.
/// The term can be anything and will be percent-decoded before being considered by the filter.
///
/// The valid operands are: `eq`, `neq`, `gt`, `gte`, `lt`, `lte`, `like`, 'in', 'nin'.
static FILTER_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\A(eq|neq|gt|gte|lt|lte|like|in|nin):(.*)\z").unwrap());

/// Matches a family parameter in the form `$family[$param]`.
///
/// The following families are supported:
///
/// - `filter[$field_name]`
/// - `fields[$model_name]`
/// - `page[number]` and `page[size]`
static FAMILY_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    use regex_builder::ID;
    Regex::new(format!(r"\A(filter|page|fields)\[({ID})]\z").as_str()).unwrap()
});

/// Matches a possibly nested include parameter in the form `relationship.another.nested`
static INCLUDE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    use regex_builder::ID;
    Regex::new(format!(r"\A(?:{ID}\.)*{ID}\z").as_str()).unwrap()
});

/// Enumerates possible sort directions: ascending and descending
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

/// Stores information for sorting a collection
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortingAttribute<'sch> {
    pub(crate) attribute: &'sch str,
    pub(crate) direction: SortDirection,
}

/// Enumerates possible comparison operations available for filtering
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterValue {
    Equal(Attribute),
    NotEqual(Attribute),
    GreaterThan(Attribute),
    GreaterThanOrEqual(Attribute),
    LessThan(Attribute),
    LessThanOrEqual(Attribute),
    Like(Attribute),
    In(HashSet<Attribute>),
    NotIn(HashSet<Attribute>),
}

/// Stores which fields should be returned for a given model type
pub type FieldsParameters<'sch> = HashMap<&'sch str, HashSet<&'sch str>>;

/// Stores which filters should be applied for each field from the primary data
pub type FilterParameters<'sch> = HashMap<&'sch str, Vec<FilterValue>>;

/// Stores a series of terms to be searched
pub type SearchParameters<'req> = Vec<Cow<'req, str>>;

/// Represents a single node in the include tree
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncludeNode<'sch: 'req, 'req> {
    relationship: &'req str,
    descriptor: &'sch Relationship<'sch>,
    children: HashMap<&'sch str, IncludeNode<'sch, 'req>>,
}

/// Stores a series of relationship names which should be included in the final payload
pub type IncludeParameters<'sch, 'req> = HashMap<&'sch str, IncludeNode<'sch, 'req>>;

/// Stores how the primary collection should be sorted
pub type SortParameters<'sch> = Vec<SortingAttribute<'sch>>;

/// Stores how the primary collection should be paged
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageParameters {
    pub(crate) number: u32,
    pub(crate) size: u32,
}

impl Default for PageParameters {
    /// Constructs a new paging object, defaulting to first page with 20 records per page.
    fn default() -> Self {
        Self {
            number: 1,
            size: 20,
        }
    }
}

/// Auxiliary struct to collect model schemas that should be loaded for all the requested
/// information to be served
pub type ModelsToLoad<'sch> = HashMap<&'sch str, &'sch TableSchema<'sch>>;

/// Stores all possible query parameters received.
/// Those parameters will be used later to determine which and how data should be loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryParameters<'sch, 'req> {
    pub schema: &'sch TableSchema<'sch>,
    pub fields: FieldsParameters<'sch>,
    pub include: IncludeParameters<'sch, 'req>,
    pub filter: Option<FilterParameters<'sch>>,
    pub search: Option<SearchParameters<'req>>,
    pub sort: Option<SortParameters<'sch>>,
    pub page: Option<PageParameters>,
}

impl<'sch, 'req> QueryParameters<'sch, 'req>  {
    pub fn new(schema: &'sch TableSchema<'sch>) -> Self {
        Self {
            schema,
            fields: FieldsParameters::new(),
            include: IncludeParameters::new(),
            filter: None,
            search: None,
            sort: None,
            page: None,
        }
    }

    /// Main entry point.
    /// Attempts to extract supported parameters from the provided URI.
    /// Any parsing errors (encoding errors, failed validations etc.) will cause the function to
    /// return an `Err`.
    pub fn parse<'reg, Adapter: AdapterInterface>(
        uri: &'req Uri,
        schema: &'sch TableSchema<'sch>,
        registry: &'reg Registry<'sch, Adapter>
    ) -> Result<QueryParameters<'sch, 'req>, Error> {
        let mut query_parameters = QueryParameters::new(schema);

        if let Some(query) = uri.query() {
            query_parameters.parse_query(query, schema, registry)?;
        } else {
            query_parameters.fields.insert(schema.name, HashSet::from_iter(schema.fields()));
        }

        Ok(query_parameters)
    }


    pub fn derive<'reg, Adapter: AdapterInterface>(&self, relationship: &str, registry: &'reg Registry<'sch, Adapter>) -> Result<Self, Error> {
        let include =
            self.include
                .get(relationship)
                .ok_or_else(|| Error::SchemaValidationFailure {
                    schema: self.schema.name.to_string(),
                    attribute: relationship.to_string(),
                    message: "Invalid relationship requested".to_string(),
                })?;

        let schema = registry
            .table(include.descriptor.related_resource().resource)?
            .schema();

        Ok(Self {
            fields: self.fields.clone(),
            include: include.children.clone(),
            ..QueryParameters::new(schema)
        })
    }

    pub fn is_requested(&self, field: &str) -> bool {
        match self.fields.get(self.schema.name) {
            Some(fields) => fields.contains(field),
            None => false,
        }
    }

    pub fn is_included(&self, relationship: &str) -> bool {
        self.include.contains_key(relationship)
    }

    pub fn should_load(&self, relationship: &str) -> bool {
        self.is_requested(relationship) || self.is_included(relationship)
    }

    pub fn relationships_to_load(&self)
        -> impl Iterator<Item = &'sch (&'sch str, Relationship<'sch>)>
    {
        self.schema
            .relationships
            .iter()
            .filter(|(relationship, _)| self.should_load(*relationship))
    }

    fn parse_fields<'reg, Adapter: AdapterInterface>(&mut self, model: &'req str, fields: &'req str, registry: &'reg Registry<'sch, Adapter>) -> Result<(), Error> {
        if fields.is_empty() {
            return Ok(());
        }

        let schema = registry.table(model)?.schema();
        let schema_fields = schema.fields().collect::<HashSet<_>>();

        let model_fields = fields
            .split(',')
            .map(|field| {
                if let Some(&field) = schema_fields.get(field) {
                    Ok(field)
                } else {
                    Err(Error::SchemaValidationFailure {
                        schema: schema.name.to_string(),
                        attribute: field.to_string(),
                        message: "Requested field is invalid".to_string(),
                    })
                }
            })
            .collect::<Result<Vec<_>, Error>>()?;

        for field in model_fields {
            self.fields
                .entry(schema.name)
                .or_insert(HashSet::new())
                .insert(field);
        }

        Ok(())
    }
    fn decode_str(value: &'req str) -> Result<Cow<'req, str>, Error> {
        decode(value).map_err(|_| InvalidEncodingFailure)
    }

    fn parse_attribute(value: &'req str, kind: &'sch AttributeType) -> Result<Attribute, Error> {
        let value = Self::decode_str(value)?;
        Attribute::parse(&value, *kind)
    }

    fn parse_attribute_set(value: &'req str, kind: &'sch AttributeType) -> Result<HashSet<Attribute>, Error> {
        value
            .split(",")
            .map(|value| Self::parse_attribute(value, kind))
            .collect()
    }

    fn parse_filter(&mut self, attribute: &'req str, entries: &'req str, schema: &'sch TableSchema)
        -> Result<(), Error>
    {
        let (attribute, kind) = schema.attributes
            .iter()
            .find(|(name, _)| *name == attribute)
            .ok_or_else(|| SchemaValidationFailure {
                schema: schema.name.to_string(),
                attribute: attribute.to_string(),
                message: "Attempted to filter on an unknown attribute".to_string(),
            })?;

        let filter = entries
            .split(",")
            .map(|entry| {
                let result = FILTER_REGEX.captures(entry).map(|c| c.extract());

                if let Some((_, [operator, value])) = result {
                    use FilterValue::*;
                    let filter_value = match operator {
                        "eq" => Equal(Self::parse_attribute(value, kind)?),
                        "neq" => NotEqual(Self::parse_attribute(value, kind)?),
                        "gt" => GreaterThan(Self::parse_attribute(value, kind)?),
                        "gte" => GreaterThanOrEqual(Self::parse_attribute(value, kind)?),
                        "lt" => LessThan(Self::parse_attribute(value, kind)?),
                        "lte" => LessThanOrEqual(Self::parse_attribute(value, kind)?),
                        "like" => Like(Self::parse_attribute(value, kind)?),
                        "in" => In(Self::parse_attribute_set(value, kind)?),
                        "nin" => NotIn(Self::parse_attribute_set(value, kind)?),
                        _ => Err(Error::ParseParameterFailure {
                            parameter: format!("filter[{attribute}]"),
                            message: format!("Invalid filter operator: '{operator}'"),
                        })?,
                    };

                    Ok(filter_value)
                } else {
                    Err(Error::ParseParameterFailure {
                        parameter: format!("filter[{attribute}]"),
                        message: format!("Invalid filter entry: '{entry}'"),
                    })
                }
            })
            .collect::<Result<Vec<_>, Error>>()?;

        self.filter
            .get_or_insert_default()
            .insert(attribute, filter);

        Ok(())
    }

    fn parse_search(&mut self, values: &'req str) -> Result<(), Error> {
        if !values.is_empty() {
            self.search = Some(
                values
                    .split(",")
                    .filter(|entry| !entry.is_empty())
                    .map(decode)
                    .collect::<Result<Vec<_>, _>>()?,
            )
        }

        Ok(())
    }

    fn parse_include<'reg, Adapter: AdapterInterface>(
        &mut self,
        include: &str,
        models: &mut HashMap<&'sch str, &'sch TableSchema<'sch>>,
        schema: &'sch TableSchema<'sch>,
        registry: &'reg Registry<'sch, Adapter>
    ) -> Result<(), Error> {
        if !include.is_empty() {
            let mut relationship;
            let mut rest = Some(include);
            let mut scope = &mut self.include;
            let mut schema = schema;

            while let Some(path) = rest {
                (relationship, rest) = match path.split_once(".") {
                    Some((relationship, rest)) => (relationship, Some(rest)),
                    None => (path, None),
                };

                let (relationship, descriptor) = schema
                    .relationships
                    .iter()
                    .find(|(r, _)| relationship == *r)
                    .ok_or_else(|| Error::SchemaValidationFailure {
                        schema: schema.name.to_string(),
                        attribute: relationship.to_string(),
                        message: "Invalid relationship requested".to_string(),
                    })?;

                schema = registry
                    .table(descriptor.related_resource().resource)?
                    .schema();

                models.insert(schema.name, schema);

                scope = &mut scope
                    .entry(relationship)
                    .or_insert(IncludeNode {
                        relationship,
                        descriptor,
                        children: HashMap::new(),
                    })
                    .children;
            }
        }

        Ok(())
    }

    fn parse_sort(&mut self, entries: &'req str,schema: &'sch TableSchema<'sch>) -> Result<(), Error> {
        let attributes = schema.attributes
            .iter()
            .map(|(name, _)| *name)
            .collect::<HashSet<_>>();

        self.sort = Some(
            entries
                .split(",")
                .map(|entry| {
                    let result = SORT_REGEX.captures(entry).map(|c| c.extract());
                    if let Some((_, [sign, attribute])) = result {
                        let attribute = attributes.get(attribute)
                            .ok_or_else(|| SchemaValidationFailure {
                                schema: schema.name.to_string(),
                                attribute: attribute.to_string(),
                                message: "Invalid attribute to sort".to_string(),

                            })?;

                        Ok(SortingAttribute {
                            attribute,
                            direction: match sign {
                                "-" => SortDirection::Descending,
                                "" | "+" => SortDirection::Ascending,
                                _ => unreachable!(),
                            },
                        })
                    } else {
                        Err(Error::ParseParameterFailure {
                            parameter: "sort".to_string(),
                            message: format!("Invalid sorting entry: '{entry}'"),
                        })
                    }
                })
                .collect::<Result<Vec<_>, Error>>()?,
        );

        Ok(())
    }

    fn parse_page(&mut self, property: &str, value: &str) -> Result<(), Error> {
        let value = value
            .parse::<NonZeroU32>()
            .map_err(|_| Error::ParseParameterFailure {
                parameter: format!("page[{property}]"),
                message: format!("Invalid numeric value: '{value}'"),
            })?
            .get();

        let page = self.page.get_or_insert_default();
        match property {
            "number" => page.number = value,
            "size" => page.size = value,
            _ => Err(Error::ParseParameterFailure {
                parameter: format!("page[{property}]"),
                message: format!("Invalid page property: '{property}'"),
            })?,
        };

        Ok(())
    }

    fn parse_query<'reg, Adapter: AdapterInterface>(
        &mut self,
        query: &'req str,
        schema: &'sch TableSchema<'sch>,
        registry: &'reg Registry<'sch, Adapter>
    ) -> Result<(), Error> {
        let mut models_to_load = HashMap::new();

        for (entry, split) in query
            .split('&')
            .filter(|s| !s.is_empty())
            .map(|entry| (entry, entry.split_once('=')))
        {
            match split.ok_or_else(|| Error::ParseParameterFailure {
                parameter: entry.to_string(),
                message: format!("Invalid query entry: '{entry}'"),
            })? {
                ("search", value) => self.parse_search(value)?,
                ("include", include) => self.parse_include(include, &mut models_to_load, schema, registry)?,
                ("sort", sort) => self.parse_sort(sort, schema)?,
                (key, value) => match FAMILY_REGEX.captures(key).map(|c| c.extract()) {
                    Some((_, ["fields", model])) => self.parse_fields(model, value, registry)?,
                    Some((_, ["filter", field])) => self.parse_filter(field, value, schema)?,
                    Some((_, ["page", property])) => self.parse_page(property, value)?,
                    Some((parameter, [..])) => Err(Error::ParseParameterFailure {
                        parameter: parameter.to_string(),
                        message: "Unexpected parameter provided".to_string(),
                    })?,
                    None => Err(ParseParameterFailure {
                        parameter: key.to_string(),
                        message: "Unknown parameter provided".to_string(),
                    })?,
                },
            }
        }

        Ok(())
    }
}
//
// #[cfg(test)]
// mod tests {
//     use super::*;
//     use crate::database::schema::tests::PRODUCTS_SCHEMA;
//
//     fn mock_uri(query: &str) -> Uri {
//         format!("http://localhost:8000/resource?{}", query)
//             .parse::<Uri>()
//             .unwrap()
//     }
//
//     #[test]
//     fn test_new() {
//         let params = QueryParameters::new(&PRODUCTS_SCHEMA);
//         assert_eq!(params.schema, &PRODUCTS_SCHEMA);
//         assert!(params.fields.is_empty());
//         assert!(params.include.is_empty());
//         assert!(params.filter.is_none());
//         assert!(params.sort.is_none());
//         assert!(params.page.is_none());
//         assert!(params.search.is_none());
//     }
//
//     #[test]
//     fn test_parse_fields() {
//         let uri = mock_uri("fields[model1]=col1,col2,col3&fields[model2]=col4");
//         let params = QueryParameters::parse(&uri).unwrap();
//         let fields = params.fields.unwrap();
//         assert_eq!(
//             fields,
//             IndexMap::from([
//                 ("model1".to_string(), vec!["col1", "col2", "col3"]),
//                 ("model2".to_string(), vec!["col4"]),
//             ])
//         );
//     }
//
//     #[test]
//     fn test_parse_sort() {
//         let uri = mock_uri("sort=-col1,col2");
//         let params = QueryParameters::parse(&uri).unwrap();
//         let sort = params.sort.unwrap();
//         assert_eq!(sort.len(), 2);
//         assert_eq!(sort[0].attribute, "col1");
//         assert_eq!(sort[0].direction, SortDirection::Descending);
//         assert_eq!(sort[1].attribute, "col2");
//         assert_eq!(sort[1].direction, SortDirection::Ascending);
//     }
//
//     #[test]
//     fn test_parse_pagination() {
//         let uri = mock_uri("page[number]=2&page[size]=20");
//         let params = QueryParameters::parse(&uri).unwrap();
//         assert_eq!(
//             params.page,
//             Some(PageParameters {
//                 number: 2,
//                 size: 20
//             })
//         );
//     }
//
//     #[test]
//     fn test_parse_filter_eq() {
//         let uri = mock_uri("filter[col1]=eq:value1&filter[col2]=neq:value2");
//         let params = QueryParameters::parse(&uri).unwrap();
//
//         assert!(params.filter.is_some());
//
//         let filters = params.filter.unwrap();
//         assert_eq!(filters["col1"][0], FilterValue::Equal("value1".to_string()));
//         assert_eq!(
//             filters["col2"][0],
//             FilterValue::NotEqual("value2".to_string())
//         );
//     }
//
//     #[test]
//     fn test_parse_mixed_uri() {
//         let uri = mock_uri(
//             "fields[my_model]=col1,col2&sort=-col1&filter[col1]=gt:10&page[number]=3&page[size]=15",
//         );
//         let params = QueryParameters::parse(&uri).unwrap();
//
//         // Fields check
//         let fields = params.fields.unwrap();
//         assert_eq!(
//             fields,
//             IndexMap::from([("my_model".to_string(), vec!["col1", "col2"]),])
//         );
//
//         // Sort check
//         let sort = params.sort.unwrap();
//         assert_eq!(sort[0].attribute, "col1");
//         assert_eq!(sort[0].direction, SortDirection::Descending);
//
//         // Filter check
//         let filters = params.filter.unwrap();
//         assert_eq!(
//             filters["col1"][0],
//             FilterValue::GreaterThan("10".to_string())
//         );
//
//         // Pagination check
//         assert_eq!(
//             params.page,
//             Some(PageParameters {
//                 number: 3,
//                 size: 15
//             })
//         );
//     }
//
//     #[test]
//     fn test_invalid_sort_format() {
//         let uri = mock_uri("sort=--col1");
//         let params = QueryParameters::parse(&uri);
//         assert!(
//             params.is_err(),
//             "Expected parsing to fail when invalid sort field is provided"
//         );
//     }
//
//     #[test]
//     fn test_invalid_filter_format() {
//         let uri = mock_uri("filter[col1]=value1"); // Missing operator:value format
//         let params = QueryParameters::parse(&uri);
//         assert!(
//             params.is_err(),
//             "Expected parsing to fail when invalid filter is provided"
//         );
//     }
//
//     #[test]
//     fn test_parse_invalid_page_size() {
//         let uri = mock_uri("page[number]=2&page[size]=abc");
//         let params = QueryParameters::parse(&uri);
//         assert!(
//             params.is_err(),
//             "Expected parsing to fail for invalid page size"
//         );
//     }
//
//     #[test]
//     fn test_parse_empty_sort() {
//         let uri = mock_uri("sort=");
//         let params = QueryParameters::parse(&uri);
//         assert!(
//             params.is_err(),
//             "Expected parsing to fail for empty sort parameter"
//         );
//     }
//
//     #[test]
//     fn test_parse_multiple_filters_for_same_field() {
//         let uri = mock_uri("filter[col1]=eq:value1,gt:-20");
//         let params = QueryParameters::parse(&uri).unwrap();
//
//         assert!(params.filter.is_some());
//         let filters = params.filter.unwrap();
//
//         assert_eq!(filters["col1"].len(), 2);
//         assert_eq!(filters["col1"][0], FilterValue::Equal("value1".to_string()));
//         assert_eq!(
//             filters["col1"][1],
//             FilterValue::GreaterThan("-20".to_string())
//         );
//     }
//
//     #[test]
//     fn test_parse_no_filter_value() {
//         let uri = mock_uri("filter[col1]=");
//         let params = QueryParameters::parse(&uri);
//         assert!(
//             params.is_err(),
//             "Expected parsing to fail for missing filter value"
//         );
//     }
//
//     #[test]
//     fn test_parse_empty_fields() {
//         let uri = mock_uri("fields=");
//         let params = QueryParameters::parse(&uri);
//         assert!(params.is_err(), "Expected parsing to fail for empty fields");
//     }
//
//     #[test]
//     fn test_parse_invalid_field_name() {
//         let uri = mock_uri("fields=col1,invalid!field");
//         let params = QueryParameters::parse(&uri);
//         assert!(
//             params.is_err(),
//             "Expected parsing to fail for invalid field name"
//         );
//     }
//
//     #[test]
//     fn test_parse_page_only_number() {
//         let uri = mock_uri("page[number]=2");
//         let params = QueryParameters::parse(&uri).unwrap();
//         assert_eq!(
//             params.page,
//             Some(PageParameters {
//                 number: 2,
//                 size: 20
//             })
//         );
//     }
//
//     #[test]
//     fn test_parse_sort_with_invalid_characters() {
//         let uri = mock_uri("sort=col@1");
//         let params = QueryParameters::parse(&uri);
//         assert!(
//             params.is_err(),
//             "Expected parsing to fail for invalid sort field"
//         );
//     }
//
//     #[test]
//     fn test_parse_filter_with_special_characters() {
//         // 'value1&' should be URL-encoded as 'value1%26'
//         let uri = mock_uri("filter[col1]=eq:value1%26&filter[col2]=like:special%3Avalue");
//         let params = QueryParameters::parse(&uri).unwrap();
//
//         let filters = params.filter.unwrap();
//
//         assert_eq!(
//             filters["col1"][0],
//             FilterValue::Equal("value1&".to_string())
//         );
//         assert_eq!(
//             filters["col2"][0],
//             FilterValue::Like("special:value".to_string())
//         );
//     }
//
//     #[test]
//     fn test_search_without_value() {
//         let uri = mock_uri("search=");
//         let params = QueryParameters::parse(&uri).unwrap();
//
//         assert!(params.search.is_none());
//     }
//
//     #[test]
//     fn test_search_with_single_value() {
//         let uri = mock_uri("search=some-value");
//         let params = QueryParameters::parse(&uri).unwrap();
//
//         assert_eq!(params.search, Some(vec!["some-value".to_string()]));
//     }
//
//     #[test]
//     fn test_search_with_multiple_values() {
//         let uri = mock_uri("search=some-value,another-value");
//         let params = QueryParameters::parse(&uri).unwrap();
//
//         assert_eq!(
//             params.search,
//             Some(vec!["some-value".to_string(), "another-value".to_string()])
//         );
//     }
//
//     #[test]
//     fn test_parse_include_single_value() {
//         let uri = mock_uri("include=author");
//         let params = QueryParameters::parse(&uri).unwrap();
//         assert_eq!(params.include, Some(vec!["author".to_string()]));
//     }
//
//     #[test]
//     fn test_parse_include_multiple_values() {
//         let uri = mock_uri("include=author,comments,tags");
//         let params = QueryParameters::parse(&uri).unwrap();
//         assert_eq!(
//             params.include,
//             Some(vec![
//                 "author".to_string(),
//                 "comments".to_string(),
//                 "tags".to_string()
//             ])
//         );
//     }
//
//     #[test]
//     fn test_parse_include_empty() {
//         let uri = mock_uri("include=");
//         let params = QueryParameters::parse(&uri).unwrap();
//         assert!(params.include.is_none());
//     }
//
//     #[test]
//     fn test_parse_page_only_size() {
//         let uri = mock_uri("page[size]=50");
//         let params = QueryParameters::parse(&uri).unwrap();
//         assert_eq!(
//             params.page,
//             Some(PageParameters {
//                 number: 1,
//                 size: 50
//             })
//         );
//     }
//
//     #[test]
//     fn test_parse_page_invalid_property() {
//         let uri = mock_uri("page[limit]=10");
//         let params = QueryParameters::parse(&uri);
//         assert!(
//             params.is_err(),
//             "Expected parsing to fail for invalid page property"
//         );
//     }
//
//     #[test]
//     fn test_parse_page_zero_value() {
//         let uri = mock_uri("page[number]=0");
//         let params = QueryParameters::parse(&uri);
//         assert!(
//             params.is_err(),
//             "Expected parsing to fail for zero page number"
//         );
//     }
//
//     #[test]
//     fn test_parse_page_negative_value() {
//         let uri = mock_uri("page[size]=-5");
//         let params = QueryParameters::parse(&uri);
//         assert!(
//             params.is_err(),
//             "Expected parsing to fail for negative page size"
//         );
//     }
//
//     #[test]
//     fn test_parse_fields_with_hyphens() {
//         let uri = mock_uri("fields[model]=field-name,another-field");
//         let params = QueryParameters::parse(&uri).unwrap();
//         let fields = params.fields.unwrap();
//         assert_eq!(fields["model"], vec!["field-name", "another-field"]);
//     }
//
//     #[test]
//     fn test_parse_fields_with_underscores() {
//         let uri = mock_uri("fields[model]=field_name,another_field");
//         let params = QueryParameters::parse(&uri).unwrap();
//         let fields = params.fields.unwrap();
//         assert_eq!(fields["model"], vec!["field_name", "another_field"]);
//     }
//
//     #[test]
//     fn test_parse_fields_with_numbers() {
//         let uri = mock_uri("fields[model]=field1,field2name,name3");
//         let params = QueryParameters::parse(&uri).unwrap();
//         let fields = params.fields.unwrap();
//         assert_eq!(fields["model"], vec!["field1", "field2name", "name3"]);
//     }
//
//     #[test]
//     fn test_parse_fields_starting_with_hyphen() {
//         let uri = mock_uri("fields[model]=-invalidfield");
//         let params = QueryParameters::parse(&uri);
//         assert!(
//             params.is_err(),
//             "Expected parsing to fail for field starting with hyphen"
//         );
//     }
//
//     #[test]
//     fn test_parse_fields_empty_value_for_model() {
//         let uri = mock_uri("fields[model]=");
//         let params = QueryParameters::parse(&uri).unwrap();
//         assert!(params.fields.is_none());
//     }
//
//     #[test]
//     fn test_parse_search_with_encoded_spaces() {
//         let uri = mock_uri("search=hello%20world,foo%20bar");
//         let params = QueryParameters::parse(&uri).unwrap();
//         assert_eq!(
//             params.search,
//             Some(vec!["hello world".to_string(), "foo bar".to_string()])
//         );
//     }
//
//     #[test]
//     fn test_parse_include_with_encoded_values() {
//         let uri = mock_uri("include=author%2Dprofile,user%2Dposts");
//         let result = QueryParameters::parse(&uri);
//         assert!(
//             result.is_err(),
//             "Expected parsing to fail for includes with percent-encoded values"
//         );
//     }
//
//     #[test]
//     fn test_parse_filter_all_operators() {
//         let uri = mock_uri("filter[age]=gte:18,lte:65&filter[status]=neq:deleted");
//         let params = QueryParameters::parse(&uri).unwrap();
//         let filters = params.filter.unwrap();
//
//         assert_eq!(
//             filters["age"][0],
//             FilterValue::GreaterThanOrEqual("18".to_string())
//         );
//         assert_eq!(
//             filters["age"][1],
//             FilterValue::LessThanOrEqual("65".to_string())
//         );
//         assert_eq!(
//             filters["status"][0],
//             FilterValue::NotEqual("deleted".to_string())
//         );
//     }
//
//     #[test]
//     fn test_parse_filter_like_operator() {
//         let uri = mock_uri("filter[name]=like:%25john%25");
//         let params = QueryParameters::parse(&uri).unwrap();
//         let filters = params.filter.unwrap();
//
//         assert_eq!(filters["name"][0], FilterValue::Like("%john%".to_string()));
//     }
//
//     #[test]
//     fn test_parse_sort_with_explicit_plus() {
//         let uri = mock_uri("sort=+col1,col2");
//         let params = QueryParameters::parse(&uri).unwrap();
//         let sort = params.sort.unwrap();
//
//         assert_eq!(sort[0].attribute, "col1");
//         assert_eq!(sort[0].direction, SortDirection::Ascending);
//         assert_eq!(sort[1].attribute, "col2");
//         assert_eq!(sort[1].direction, SortDirection::Ascending);
//     }
//
//     #[test]
//     fn test_parse_sort_mixed_directions() {
//         let uri = mock_uri("sort=-field1,+field2,field3,-field4");
//         let params = QueryParameters::parse(&uri).unwrap();
//         let sort = params.sort.unwrap();
//
//         assert_eq!(sort.len(), 4);
//         assert_eq!(sort[0].direction, SortDirection::Descending);
//         assert_eq!(sort[1].direction, SortDirection::Ascending);
//         assert_eq!(sort[2].direction, SortDirection::Ascending);
//         assert_eq!(sort[3].direction, SortDirection::Descending);
//     }
//
//     #[test]
//     fn test_parse_all_parameters_combined() {
//         let uri = mock_uri(
//             "fields[users]=id,name&filter[status]=eq:active&sort=-created_at&page[number]=2&page[size]=25&include=profile&search=query",
//         );
//         let params = QueryParameters::parse(&uri).unwrap();
//
//         assert!(params.fields.is_some());
//         assert!(params.filter.is_some());
//         assert!(params.sort.is_some());
//         assert!(params.page.is_some());
//         assert!(params.include.is_some());
//         assert!(params.search.is_some());
//     }
//
//     #[test]
//     fn test_parse_missing_equals_sign() {
//         let uri = mock_uri("sortcol1");
//         let params = QueryParameters::parse(&uri);
//         assert!(
//             params.is_err(),
//             "Expected parsing to fail for malformed query"
//         );
//     }
//
//     #[test]
//     fn test_parse_unknown_parameter() {
//         let uri = mock_uri("unknown_param=value");
//         let params = QueryParameters::parse(&uri);
//         assert!(
//             params.is_err(),
//             "Expected parsing to fail for unknown parameter"
//         );
//     }
//
//     #[test]
//     fn test_parse_filter_missing_colon() {
//         let uri = mock_uri("filter[col1]=eqvalue");
//         let params = QueryParameters::parse(&uri);
//         assert!(
//             params.is_err(),
//             "Expected parsing to fail for filter without colon separator"
//         );
//     }
// }
