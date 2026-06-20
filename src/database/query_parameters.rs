//! Defines a data structure suitable for storing JSON:API-relevant query parameters.
//! Also provides functions to extract those parameters from a query string, which is the only
//! supported construction method.

use super::error::Error;
use crate::database::adapters::Adapter as AdapterInterface;
use crate::database::attributes::Attribute;
use crate::database::error::Error::{
    InvalidEncodingFailure, ParseParameterFailure, SchemaValidationFailure,
};
use crate::database::registry::Registry;
use crate::database::schema::{AttributeType, Relationship, TableSchema};
use crate::http_wrappers::Uri;
use indexmap::{IndexMap, IndexSet};
use regex::Regex;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::{num::NonZeroU32, sync::LazyLock};
use urlencoding::decode;

mod regex_builder {
    /// Generic pattern for identifiers -- model names, field names and relationship names
    pub(crate) static ID: &str = r"[a-zA-Z](?:[-_]*[a-zA-Z0-9]+)*";
}

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
    In(IndexSet<Attribute>),
    NotIn(IndexSet<Attribute>),
}

/// Stores which fields should be returned for a given model type
pub type FieldsParameters<'sch> = IndexMap<&'sch str, IndexSet<&'sch str>>;

/// Stores which filters should be applied for each field from the primary data
pub type FilterParameters<'sch> = IndexMap<&'sch str, Vec<FilterValue>>;

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
pub type ModelsToSerialise<'sch> = HashMap<&'sch str, &'sch TableSchema<'sch>>;

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

impl<'sch, 'req> QueryParameters<'sch, 'req> {
    pub fn new(schema: &'sch TableSchema<'sch>) -> Self {
        Self {
            schema,
            fields: FieldsParameters::from_iter([(schema.name, schema.fields().collect())]),
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
    pub fn parse<Adapter: AdapterInterface>(
        uri: &'req Uri,
        schema: &'sch TableSchema<'sch>,
        registry: &Registry<'sch, Adapter>,
    ) -> Result<QueryParameters<'sch, 'req>, Error> {
        if let Some(query) = uri.query() {
            let mut query_parameters = Self {
                schema,
                fields: FieldsParameters::new(),
                include: IncludeParameters::new(),
                filter: None,
                search: None,
                sort: None,
                page: None,
            };
            query_parameters.parse_query(query, schema, registry)?;

            Ok(query_parameters)
        } else {
            Ok(QueryParameters::new(schema))
        }
    }

    pub fn derive<Adapter: AdapterInterface>(
        &self,
        relationship: &str,
        registry: &Registry<'sch, Adapter>,
    ) -> Result<Self, Error> {
        let include =
            self.include
                .get(relationship)
                .ok_or_else(|| Error::SchemaValidationFailure {
                    schema: self.schema.name.to_string(),
                    attribute: relationship.to_string(),
                    message: "Invalid relationship requested".to_string(),
                })?;

        let schema = registry.schema(include.descriptor.related_resource().resource)?;

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

    pub fn relationships_to_load(
        &self,
    ) -> impl Iterator<Item = &'sch (&'sch str, Relationship<'sch>)> {
        self.schema
            .relationships
            .iter()
            .filter(|(relationship, _)| self.should_load(relationship))
    }

    /// Completes `self.fields` so that every model the request will touch can be queried.
    ///
    /// Runs in two phases over the *serialised* models -- the primary resource plus everything
    /// reachable through `include`, which are exactly the contexts the data loader recurses into:
    ///
    /// 1. each serialised model that the request did not pin explicitly receives its full default
    ///    fieldset, so included resources are presented in full;
    /// 2. for every `HasOne`/`HasMany` out of a serialised model, the join key (`keys.related`) is
    ///    forced into the *related* model's fieldset -- creating a minimal, join-key-only entry for
    ///    link-only targets. This key lives on a table the query builder never sees on its own (it
    ///    is scoped to a single schema), so it has to be planted here. `BelongsTo` needs nothing:
    ///    its key (`keys.own`) sits on the owning table and the builder already emits it when it
    ///    expands the relationship name, and the related side is that table's primary key, which is
    ///    always selected.
    ///
    /// Because the keys are forced for every relationship regardless of whether it ends up loaded,
    /// no `should_load`/include-context bookkeeping is needed here.
    fn discover_fields_for_remaining_models(
        &mut self,
        models_to_serialise: ModelsToSerialise<'sch>,
    ) {
        for (&model, &schema) in &models_to_serialise {
            self.fields
                .entry(model)
                .or_insert_with(|| schema.fields().collect());
        }

        for (_, schema) in models_to_serialise {
            for (_, relationship) in schema.relationships {
                use Relationship::*;
                if let HasOne(related) | HasMany(related) = relationship {
                    self.fields
                        .entry(related.resource)
                        .or_default()
                        .insert(related.keys.related);
                }
            }
        }
    }

    fn parse_fields<Adapter: AdapterInterface>(
        &mut self,
        model: &'req str,
        fields: &'req str,
        registry: &Registry<'sch, Adapter>,
    ) -> Result<(), Error> {
        if fields.is_empty() {
            return Ok(());
        }

        let schema = registry.schema(model)?;
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
            self.fields.entry(schema.name).or_default().insert(field);
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

    fn parse_attribute_set(
        value: &'req str,
        kind: &'sch AttributeType,
    ) -> Result<IndexSet<Attribute>, Error> {
        value
            .split(",")
            .map(|value| Self::parse_attribute(value, kind))
            .collect()
    }

    fn parse_filter(
        &mut self,
        attribute: &'req str,
        entries: &'req str,
        schema: &'sch TableSchema,
    ) -> Result<(), Error> {
        let (attribute, kind) = schema
            .attributes
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

    fn parse_include<Adapter: AdapterInterface>(
        &mut self,
        include: &str,
        models: &mut HashMap<&'sch str, &'sch TableSchema<'sch>>,
        schema: &'sch TableSchema<'sch>,
        registry: &Registry<'sch, Adapter>,
    ) -> Result<(), Error> {
        if !include.is_empty() {
            for include in include.split(",") {
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

                    schema = registry.schema(descriptor.related_resource().resource)?;

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
        }

        Ok(())
    }

    fn parse_sort(
        &mut self,
        entries: &'req str,
        schema: &'sch TableSchema<'sch>,
    ) -> Result<(), Error> {
        let attributes = schema
            .attributes
            .iter()
            .map(|(name, _)| *name)
            .collect::<HashSet<_>>();

        self.sort = Some(
            entries
                .split(",")
                .map(|entry| {
                    let result = SORT_REGEX.captures(entry).map(|c| c.extract());
                    if let Some((_, [sign, attribute])) = result {
                        let attribute =
                            attributes
                                .get(attribute)
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

    fn parse_query<Adapter: AdapterInterface>(
        &mut self,
        query: &'req str,
        schema: &'sch TableSchema<'sch>,
        registry: &Registry<'sch, Adapter>,
    ) -> Result<(), Error> {
        let mut models_to_serialise = HashMap::from_iter([(schema.name, schema)]);

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
                ("include", include) => {
                    self.parse_include(include, &mut models_to_serialise, schema, registry)?
                }
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

        self.discover_fields_for_remaining_models(models_to_serialise);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::adapters::SqliteAdapter;
    use crate::database::adapters::sqlite::Pool;
    use crate::database::registry::Registry as DatabaseRegistry;
    use crate::database::schema::{IdentifierType, PrimaryKey, RelatedResource, RelationshipKeys};

    type Registry = DatabaseRegistry<'static, SqliteAdapter>;

    static ARTICLES: TableSchema = TableSchema {
        name: "articles",
        primary_key: PrimaryKey {
            name: "id",
            kind: IdentifierType::Integer,
        },
        attributes: &[
            ("title", AttributeType::Text),
            ("views", AttributeType::Integer),
            ("published", AttributeType::Boolean),
            ("rating", AttributeType::Float),
            ("created_at", AttributeType::DateTime),
        ],
        foreign_keys: &[("author_id", AttributeType::Integer)],
        relationships: &[
            (
                "author",
                Relationship::BelongsTo(RelatedResource {
                    resource: "users",
                    keys: RelationshipKeys {
                        own: "author_id",
                        related: "id",
                    },
                }),
            ),
            (
                "comments",
                Relationship::HasMany(RelatedResource {
                    resource: "comments",
                    keys: RelationshipKeys {
                        own: "id",
                        related: "article_id",
                    },
                }),
            ),
        ],
        text_index: false,
    };

    static USERS: TableSchema = TableSchema {
        name: "users",
        primary_key: PrimaryKey {
            name: "id",
            kind: IdentifierType::Integer,
        },
        attributes: &[("name", AttributeType::Text)],
        foreign_keys: &[],
        relationships: &[(
            "articles",
            Relationship::HasMany(RelatedResource {
                resource: "articles",
                keys: RelationshipKeys {
                    own: "id",
                    related: "author_id",
                },
            }),
        )],
        text_index: false,
    };

    static COMMENTS: TableSchema = TableSchema {
        name: "comments",
        primary_key: PrimaryKey {
            name: "id",
            kind: IdentifierType::Integer,
        },
        attributes: &[("body", AttributeType::Text)],
        foreign_keys: &[("article_id", AttributeType::Integer)],
        relationships: &[(
            "article",
            Relationship::BelongsTo(RelatedResource {
                resource: "articles",
                keys: RelationshipKeys {
                    own: "article_id",
                    related: "id",
                },
            }),
        )],
        text_index: false,
    };

    static SCHEMAS: [&TableSchema; 3] = [&ARTICLES, &USERS, &COMMENTS];

    fn registry() -> Registry {
        DatabaseRegistry::try_new(Pool::memory().unwrap(), &SCHEMAS).unwrap()
    }

    fn mock_uri(query: &str) -> Uri {
        format!("http://localhost:8000/articles?{}", query)
            .parse::<Uri>()
            .unwrap()
    }

    fn parse<'req>(registry: &Registry, uri: &'req Uri) -> QueryParameters<'static, 'req> {
        QueryParameters::parse(uri, &ARTICLES, registry).unwrap()
    }

    fn parse_err(query: &str) -> Error {
        let registry = registry();
        let uri = mock_uri(query);
        QueryParameters::parse(&uri, &ARTICLES, &registry).expect_err("expected parsing to fail")
    }

    // --- Construction ---

    #[test]
    fn test_new() {
        let params = QueryParameters::new(&ARTICLES);

        assert_eq!(params.schema, &ARTICLES);
        assert_eq!(
            params.fields["articles"],
            ARTICLES.fields().collect::<IndexSet<_>>()
        );
        assert!(params.include.is_empty());
        assert!(params.filter.is_none());
        assert!(params.sort.is_none());
        assert!(params.page.is_none());
        assert!(params.search.is_none());
    }

    #[test]
    fn test_parse_empty_query() {
        let registry = registry();
        let uri = "http://localhost:8000/articles".parse::<Uri>().unwrap();
        let params = QueryParameters::parse(&uri, &ARTICLES, &registry).unwrap();

        assert_eq!(params, QueryParameters::new(&ARTICLES));
    }

    // --- Fields ---

    #[test]
    fn test_parse_fields_single_model() {
        let registry = registry();
        let uri = mock_uri("fields[articles]=title,views");
        let params = parse(&registry, &uri);

        assert_eq!(
            params.fields["articles"],
            IndexSet::from(["title", "views"])
        );
    }

    #[test]
    fn test_parse_fields_preserves_request_order() {
        let registry = registry();
        let uri = mock_uri("fields[articles]=views,title");
        let params = parse(&registry, &uri);

        assert_eq!(
            params.fields["articles"]
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            vec!["views", "title"]
        );
    }

    #[test]
    fn test_parse_fields_multiple_models() {
        let registry = registry();
        let uri = mock_uri("fields[articles]=title&fields[users]=name");
        let params = parse(&registry, &uri);

        assert_eq!(params.fields["articles"], IndexSet::from(["title"]));
        assert_eq!(params.fields["users"], IndexSet::from(["name"]));
    }

    #[test]
    fn test_parse_fields_accepts_relationship_names() {
        let registry = registry();
        let uri = mock_uri("fields[articles]=title,author");
        let params = parse(&registry, &uri);

        assert_eq!(
            params.fields["articles"],
            IndexSet::from(["title", "author"])
        );
    }

    #[test]
    fn test_parse_fields_empty_value_defaults_to_all() {
        let registry = registry();
        let uri = mock_uri("fields[articles]=");
        let params = parse(&registry, &uri);

        assert_eq!(
            params.fields["articles"],
            ARTICLES.fields().collect::<IndexSet<_>>()
        );
    }

    #[test]
    fn test_parse_fields_invalid_field() {
        assert!(matches!(
            parse_err("fields[articles]=nonexistent"),
            Error::SchemaValidationFailure { .. }
        ));
    }

    #[test]
    fn test_parse_fields_unknown_model() {
        assert!(matches!(
            parse_err("fields[ghosts]=title"),
            Error::UnknownSchema { .. }
        ));
    }

    // --- Filters ---

    #[test]
    fn test_parse_filter_eq() {
        let registry = registry();
        let uri = mock_uri("filter[title]=eq:hello");
        let params = parse(&registry, &uri);

        assert_eq!(
            params.filter.unwrap()["title"],
            vec![FilterValue::Equal(Attribute::Text("hello".to_string()))]
        );
    }

    #[test]
    fn test_parse_filter_all_operators() {
        let registry = registry();
        let uri = mock_uri("filter[views]=eq:1,neq:2,gt:3,gte:4,lt:5,lte:6");
        let params = parse(&registry, &uri);

        use FilterValue::*;
        assert_eq!(
            params.filter.unwrap()["views"],
            vec![
                Equal(Attribute::Integer(1)),
                NotEqual(Attribute::Integer(2)),
                GreaterThan(Attribute::Integer(3)),
                GreaterThanOrEqual(Attribute::Integer(4)),
                LessThan(Attribute::Integer(5)),
                LessThanOrEqual(Attribute::Integer(6)),
            ]
        );
    }

    #[test]
    fn test_parse_filter_multiple_fields_preserve_order() {
        let registry = registry();
        let uri = mock_uri("filter[title]=eq:x&filter[views]=gt:5");
        let params = parse(&registry, &uri);

        let filter = params.filter.unwrap();
        assert_eq!(
            filter.keys().copied().collect::<Vec<_>>(),
            vec!["title", "views"]
        );
    }

    #[test]
    fn test_parse_filter_typed_values() {
        let registry = registry();
        let uri = mock_uri("filter[published]=eq:true&filter[rating]=gt:4.5");
        let params = parse(&registry, &uri);

        let filter = params.filter.unwrap();
        assert_eq!(
            filter["published"],
            vec![FilterValue::Equal(Attribute::Boolean(true))]
        );
        assert_eq!(
            filter["rating"],
            vec![FilterValue::GreaterThan(Attribute::Float(4.5))]
        );
    }

    #[test]
    fn test_parse_filter_datetime_value() {
        let registry = registry();
        let uri = mock_uri("filter[created_at]=gte:2020-01-01T00:00:00Z");
        let params = parse(&registry, &uri);

        assert!(matches!(
            params.filter.unwrap()["created_at"][0],
            FilterValue::GreaterThanOrEqual(Attribute::DateTime(_))
        ));
    }

    #[test]
    fn test_parse_filter_in() {
        let registry = registry();
        let uri = mock_uri("filter[views]=in:42");
        let params = parse(&registry, &uri);

        assert_eq!(
            params.filter.unwrap()["views"],
            vec![FilterValue::In(IndexSet::from([Attribute::Integer(42)]))]
        );
    }

    #[test]
    fn test_parse_filter_like_decodes_value() {
        let registry = registry();
        let uri = mock_uri("filter[title]=like:%25john%25");
        let params = parse(&registry, &uri);

        assert_eq!(
            params.filter.unwrap()["title"],
            vec![FilterValue::Like(Attribute::Text("%john%".to_string()))]
        );
    }

    #[test]
    fn test_parse_filter_decodes_special_characters() {
        let registry = registry();
        let uri = mock_uri("filter[title]=eq:value%26");
        let params = parse(&registry, &uri);

        assert_eq!(
            params.filter.unwrap()["title"],
            vec![FilterValue::Equal(Attribute::Text("value&".to_string()))]
        );
    }

    #[test]
    fn test_parse_filter_unknown_attribute() {
        assert!(matches!(
            parse_err("filter[ghost]=eq:1"),
            Error::SchemaValidationFailure { .. }
        ));
    }

    #[test]
    fn test_parse_filter_on_relationship_is_rejected() {
        assert!(matches!(
            parse_err("filter[author]=eq:1"),
            Error::SchemaValidationFailure { .. }
        ));
    }

    #[test]
    fn test_parse_filter_on_foreign_key_is_rejected() {
        assert!(matches!(
            parse_err("filter[author_id]=eq:1"),
            Error::SchemaValidationFailure { .. }
        ));
    }

    #[test]
    fn test_parse_filter_invalid_value_for_type() {
        assert!(matches!(
            parse_err("filter[views]=eq:not_a_number"),
            Error::InvalidAttributeConversion { .. }
        ));
    }

    #[test]
    fn test_parse_filter_invalid_datetime_value() {
        assert!(matches!(
            parse_err("filter[created_at]=eq:not_a_date"),
            Error::InvalidAttributeConversion { .. }
        ));
    }

    #[test]
    fn test_parse_filter_missing_colon() {
        assert!(matches!(
            parse_err("filter[title]=eqhello"),
            Error::ParseParameterFailure { .. }
        ));
    }

    #[test]
    fn test_parse_filter_invalid_operator() {
        assert!(matches!(
            parse_err("filter[title]=foo:bar"),
            Error::ParseParameterFailure { .. }
        ));
    }

    #[test]
    fn test_parse_filter_empty_value() {
        assert!(matches!(
            parse_err("filter[title]="),
            Error::ParseParameterFailure { .. }
        ));
    }

    // --- Sort ---

    #[test]
    fn test_parse_sort_single_descending() {
        let registry = registry();
        let uri = mock_uri("sort=-title");
        let params = parse(&registry, &uri);

        let sort = params.sort.unwrap();
        assert_eq!(sort.len(), 1);
        assert_eq!(sort[0].attribute, "title");
        assert_eq!(sort[0].direction, SortDirection::Descending);
    }

    #[test]
    fn test_parse_sort_multiple_mixed() {
        let registry = registry();
        let uri = mock_uri("sort=-title,views");
        let params = parse(&registry, &uri);

        let sort = params.sort.unwrap();
        assert_eq!(sort[0].direction, SortDirection::Descending);
        assert_eq!(sort[0].attribute, "title");
        assert_eq!(sort[1].direction, SortDirection::Ascending);
        assert_eq!(sort[1].attribute, "views");
    }

    #[test]
    fn test_parse_sort_explicit_plus() {
        let registry = registry();
        let uri = mock_uri("sort=+title");
        let params = parse(&registry, &uri);

        assert_eq!(params.sort.unwrap()[0].direction, SortDirection::Ascending);
    }

    #[test]
    fn test_parse_sort_invalid_attribute() {
        assert!(matches!(
            parse_err("sort=ghost"),
            Error::SchemaValidationFailure { .. }
        ));
    }

    #[test]
    fn test_parse_sort_invalid_format() {
        assert!(matches!(
            parse_err("sort=--title"),
            Error::ParseParameterFailure { .. }
        ));
    }

    #[test]
    fn test_parse_sort_invalid_characters() {
        assert!(matches!(
            parse_err("sort=col@1"),
            Error::ParseParameterFailure { .. }
        ));
    }

    #[test]
    fn test_parse_sort_empty() {
        assert!(matches!(
            parse_err("sort="),
            Error::ParseParameterFailure { .. }
        ));
    }

    // --- Pagination ---

    #[test]
    fn test_parse_page_number_and_size() {
        let registry = registry();
        let uri = mock_uri("page[number]=2&page[size]=25");
        let params = parse(&registry, &uri);

        assert_eq!(
            params.page,
            Some(PageParameters {
                number: 2,
                size: 25
            })
        );
    }

    #[test]
    fn test_parse_page_only_number() {
        let registry = registry();
        let uri = mock_uri("page[number]=2");
        let params = parse(&registry, &uri);

        assert_eq!(
            params.page,
            Some(PageParameters {
                number: 2,
                size: 20
            })
        );
    }

    #[test]
    fn test_parse_page_only_size() {
        let registry = registry();
        let uri = mock_uri("page[size]=50");
        let params = parse(&registry, &uri);

        assert_eq!(
            params.page,
            Some(PageParameters {
                number: 1,
                size: 50
            })
        );
    }

    #[test]
    fn test_parse_page_invalid_property() {
        assert!(matches!(
            parse_err("page[limit]=10"),
            Error::ParseParameterFailure { .. }
        ));
    }

    #[test]
    fn test_parse_page_zero_value() {
        assert!(matches!(
            parse_err("page[number]=0"),
            Error::ParseParameterFailure { .. }
        ));
    }

    #[test]
    fn test_parse_page_negative_value() {
        assert!(matches!(
            parse_err("page[size]=-5"),
            Error::ParseParameterFailure { .. }
        ));
    }

    #[test]
    fn test_parse_page_non_numeric() {
        assert!(matches!(
            parse_err("page[size]=abc"),
            Error::ParseParameterFailure { .. }
        ));
    }

    // --- Search ---

    #[test]
    fn test_parse_search_single_value() {
        let registry = registry();
        let uri = mock_uri("search=some-value");
        let params = parse(&registry, &uri);

        assert_eq!(params.search, Some(vec![Cow::Borrowed("some-value")]));
    }

    #[test]
    fn test_parse_search_multiple_values() {
        let registry = registry();
        let uri = mock_uri("search=some-value,another-value");
        let params = parse(&registry, &uri);

        assert_eq!(
            params.search,
            Some(vec![
                Cow::Borrowed("some-value"),
                Cow::Borrowed("another-value")
            ])
        );
    }

    #[test]
    fn test_parse_search_decodes_spaces() {
        let registry = registry();
        let uri = mock_uri("search=hello%20world,foo%20bar");
        let params = parse(&registry, &uri);

        assert_eq!(
            params.search,
            Some(vec![Cow::Borrowed("hello world"), Cow::Borrowed("foo bar")])
        );
    }

    #[test]
    fn test_parse_search_empty_is_none() {
        let registry = registry();
        let uri = mock_uri("search=");
        let params = parse(&registry, &uri);

        assert!(params.search.is_none());
    }

    // --- Include ---

    #[test]
    fn test_parse_include_single() {
        let registry = registry();
        let uri = mock_uri("include=author");
        let params = parse(&registry, &uri);

        assert!(params.is_included("author"));
        assert!(!params.is_included("comments"));
    }

    #[test]
    fn test_parse_include_multiple() {
        let registry = registry();
        let uri = mock_uri("include=author,comments");
        let params = parse(&registry, &uri);

        assert!(params.is_included("author"));
        assert!(params.is_included("comments"));
    }

    #[test]
    fn test_parse_include_nested() {
        let registry = registry();
        let uri = mock_uri("include=comments.article");
        let params = parse(&registry, &uri);

        assert!(params.is_included("comments"));
        assert!(params.include["comments"].children.contains_key("article"));
    }

    #[test]
    fn test_parse_include_empty_is_noop() {
        let registry = registry();
        let uri = mock_uri("include=");
        let params = parse(&registry, &uri);

        assert!(params.include.is_empty());
    }

    #[test]
    fn test_parse_include_invalid_relationship() {
        assert!(matches!(
            parse_err("include=ghost"),
            Error::SchemaValidationFailure { .. }
        ));
    }

    #[test]
    fn test_parse_include_invalid_nested_relationship() {
        assert!(matches!(
            parse_err("include=comments.ghost"),
            Error::SchemaValidationFailure { .. }
        ));
    }

    #[test]
    fn test_parse_include_encoded_value_is_rejected() {
        assert!(matches!(
            parse_err("include=author%2Dprofile"),
            Error::SchemaValidationFailure { .. }
        ));
    }

    #[test]
    fn test_include_discovers_join_keys() {
        let registry = registry();
        let uri = mock_uri("include=comments");
        let params = parse(&registry, &uri);

        assert_eq!(
            params.fields["comments"],
            IndexSet::from(["body", "article", "article_id"])
        );
    }

    // --- Predicates and derivation ---

    #[test]
    fn test_is_requested() {
        let registry = registry();
        let uri = mock_uri("fields[articles]=title");
        let params = parse(&registry, &uri);

        assert!(params.is_requested("title"));
        assert!(!params.is_requested("views"));
    }

    #[test]
    fn test_relationships_to_load_respects_sparse_fieldset() {
        let registry = registry();
        let uri = mock_uri("fields[articles]=title&include=comments");
        let params = parse(&registry, &uri);

        let loaded = params
            .relationships_to_load()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>();

        assert_eq!(loaded, vec!["comments"]);
    }

    #[test]
    fn test_derive_descends_into_relationship() {
        let registry = registry();
        let uri = mock_uri("include=comments.article");
        let params = parse(&registry, &uri);

        let derived = params.derive("comments", &registry).unwrap();

        assert_eq!(derived.schema.name, "comments");
        assert!(derived.is_included("article"));
    }

    #[test]
    fn test_derive_unknown_relationship_fails() {
        let registry = registry();
        let uri = mock_uri("include=author");
        let params = parse(&registry, &uri);

        assert!(params.derive("comments", &registry).is_err());
    }

    // --- Malformed queries ---

    #[test]
    fn test_parse_missing_equals_sign() {
        assert!(matches!(
            parse_err("noequals"),
            Error::ParseParameterFailure { .. }
        ));
    }

    #[test]
    fn test_parse_unknown_parameter() {
        assert!(matches!(
            parse_err("bogus=value"),
            Error::ParseParameterFailure { .. }
        ));
    }

    #[test]
    fn test_parse_all_parameters_combined() {
        let registry = registry();
        let uri = mock_uri(
            "fields[articles]=title,views&\
             filter[published]=eq:true&\
             sort=-created_at&\
             page[number]=2&page[size]=25&\
             include=author&\
             search=query",
        );
        let params = parse(&registry, &uri);

        assert_eq!(
            params.fields["articles"],
            IndexSet::from(["title", "views", "author_id"])
        );
        assert!(params.filter.is_some());
        assert!(params.sort.is_some());
        assert_eq!(
            params.page,
            Some(PageParameters {
                number: 2,
                size: 25
            })
        );
        assert!(params.is_included("author"));
        assert_eq!(params.search, Some(vec![Cow::Borrowed("query")]));
    }
}
