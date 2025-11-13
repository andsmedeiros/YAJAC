use itertools::Itertools;
use crate::database::{
    attributes::{Attribute, Attributes},
    error::Error,
    query_builder::QueryBuilder as QueryBuilderInterface,
    query_parameters::{
        FieldsParameters,
        FilterValue,
        FilterParameters,
        PageParameters,
        QueryParameters,
        SearchParameters,
        SortDirection,
        SortParameters,
        SortingField,
    },
    schema::{AttributeType, TableSchema},
};
use std::{any::type_name, slice, str::FromStr};

struct ExtractedAttributes {
    fields: Vec<String>,
    values: Vec<Attribute>
}

impl ExtractedAttributes {
    pub(super) fn to_placeholders(&self) -> Vec<String> {
        (1..=self.fields.len())
            .into_iter()
            .map(|i| format!("?{i}"))
            .collect()
    }
}

pub type Bindings = Vec<Attribute>;

pub struct QueryBuilder<'sch> {
    schema: &'sch TableSchema<'sch>,
}

impl<'a> QueryBuilder<'a> {
    fn build_select_clause(&self, query: &mut Vec<String>) -> Result<(), Error> {
        let fields = self.fields_for_model().join(", ");
        query.extend(["SELECT".to_string(), fields]);
        Ok(())
    }

    fn build_insert_clause(&self, attributes: Attributes, query: &mut Vec<String>)
                           -> Result<Bindings, Error>
    {
        let attributes = self.extract_attributes(attributes)?;
        let placeholders = attributes.to_placeholders();

        query.extend([
            "INSERT INTO".to_string(),
            format!("{}({})", self.schema.name, attributes.fields.join(", ")),
            format!("VALUES ({})", placeholders.join(", ")),
        ]);

        Ok(attributes.values)
    }

    fn build_update_clause(&self, id: i32, attributes: Attributes, query: &mut Vec<String>)
                           -> Result<Bindings, Error>
    {
        let attributes = self.extract_attributes(attributes)?;
        let placeholders = attributes.to_placeholders();
        query.extend([ "UPDATE".to_string(), self.schema.name.to_string() ]);

        if !attributes.fields.is_empty() {
            let fields = attributes.fields
                .into_iter()
                .zip(placeholders)
                .map(|(field, placeholder)|
                    format!("{} = {}", field, placeholder)
                )
                .join(", ");
            query.extend([ "SET".to_string(), fields ]);
        }

        query.push(format!("WHERE id = ?{}", attributes.values.len() + 1));
        Ok([attributes.values.as_slice(), [Attribute::Integer(id as i64)].as_slice()].concat())
    }

    fn build_from_clause(&self, query: &mut Vec<String>) {
        query.extend(["FROM".to_string(), self.schema.name.to_string()]);
    }

    fn build_join_clause(&self, search: &Option<SearchParameters>, query: &mut Vec<String>)
                         -> Result<(), Error>
    {
        if search.is_none() {
            return Ok(());
        }

        if !self.schema.text_index {
            return Err(Error::InvalidOperation {
                schema: self.schema.name.to_string(),
                operation: "MATCH".to_string(),
                message: "table does not support full-text search".to_string()
            })
        }

        query.push(
            format!("JOIN {}_fts fts ON {}.id = fts.rowid", self.schema.name, self.schema.name)
        );

        Ok(())
    }

    fn build_where_clause(&self, filter: &Option<FilterParameters>, search: &Option<SearchParameters>, query: &mut Vec<String>)
                          -> Result<Bindings, Error>
    {
        use FilterValue::*;

        if filter.is_none() && search.is_none() {
            return Ok(Vec::new());
        }

        let mut bindings = Vec::new();
        let mut filter_query = Vec::new();
        let mut i = 1;

        query.push("WHERE".to_string());

        if let Some(values) = search {
            for value in values {
                filter_query.push(format!("{}_fts MATCH ?{}", self.schema.name, i));
                bindings.push(Attribute::Text(value.clone()));
                i += 1;
            }
        }

        if let Some(filter) = filter {
            self.validate_attributes(filter.keys())?;

            for (field, filters) in filter {
                let kind = self.schema.attribute_type(&field)?;

                for filter in filters {
                    match filter {
                        filter @ In(_) | filter @ NotIn(_) => {
                            let (operator, values) = match filter {
                                In(values) => ("IN", values),
                                NotIn(values) => ("NOT IN", values),
                                _ => unreachable!()
                            };

                            let placeholders = values
                                .iter()
                                .enumerate()
                                .map(|(pos, _)|  format!("?{}", i + pos))
                                .join(",");
                            filter_query.push(format!("{field} {operator} ({placeholders})"));

                            bindings.extend(
                                values
                                    .iter()
                                    .map(|value| Self::parse_for_field(kind.clone(), field.as_str(), value.as_str()))
                                    .collect::<Result<Vec<Attribute>, Error>>()?
                            );
                            i += values.len();
                        },
                        Like(value) => {
                            let binding = if matches!(kind, AttributeType::Text) {
                                Attribute::Text(format!("%{}%", value))
                            } else {
                                return Err(Error::SchemaValidationFailure {
                                    schema: self.schema.name.to_string(),
                                    attribute: field.clone(),
                                    message: "'LIKE' operator cannot be applied to attribute".to_string()
                                });
                            };
                            filter_query.push(format!("{field} LIKE ?{i}"));
                            bindings.push(binding);
                            i += 1;
                        },
                        filter @ _ => {
                            let parse_value = |value: &String| {
                                Self::parse_for_field(kind, field.as_str(), value.as_str())
                            };

                            let (operator, binding) = match filter {
                                Equal(value) => ("=", parse_value(value)?),
                                NotEqual(value) => ("!=", parse_value(value)?),
                                GreaterThan(value) => (">", parse_value(value)?),
                                GreaterThanOrEqual(value) => (">=", parse_value(value)?),
                                LessThan(value) => ("<", parse_value(value)?),
                                LessThanOrEqual(value) => ("<=", parse_value(value)?),
                                _ => unreachable!()
                            };

                            filter_query.push(format!("{field} {operator} ?{i}"));
                            bindings.push(binding);
                            i += 1;
                        }
                    }

                }
            }
        }

        query.push(filter_query.join(" AND ").to_string());
        Ok(bindings)
    }

    fn build_order_by_clause(&self, sort: &Option<SortParameters>, query: &mut Vec<String>)
                             -> Result<(), Error>
    {
        if let Some(fields) = sort {
            self.validate_attributes(fields.iter().map(|f| &f.field))?;

            query.push("ORDER BY".to_string());
            let mut sort_query = Vec::new();

            for SortingField { field, direction } in fields {
                let direction = match direction {
                    SortDirection::Ascending => "ASC",
                    SortDirection::Descending => "DESC",
                };
                sort_query.push(format!("{} {}", field, direction));
            }

            query.push(sort_query.join(", ").to_string());
        }

        Ok(())
    }

    fn build_limit_offset_clauses(&self, page: &Option<PageParameters>, query: &mut Vec<String>) {
        if let Some(PageParameters { number, size }) = page {
            let limit = size.to_string();
            let offset = ((number - 1) * size).to_string();

            query.extend(["LIMIT".to_string(), limit, "OFFSET".to_string(), offset]);
        }
    }

    fn build_returning_clause(&self, fields: &Option<FieldsParameters>, query: &mut Vec<String>)
                              -> Result<(), Error>
    {
        let fields = self.fields_for_model().join(", ");
        query.push(format!("RETURNING {}", fields));
        Ok(())
    }

    fn extract_attributes(&self, attributes: Attributes) -> Result<ExtractedAttributes, Error> {
        self.validate_attributes(attributes.keys())?;

        let mut fields = Vec::<String>::new();
        let mut values = Vec::<Attribute>::new();

        for (field, value) in attributes {
            fields.push(field);
            values.push(value);
        }

        Ok(ExtractedAttributes { fields, values })
    }

    fn validate_attributes<'b, I>(&self, mut attributes: I) -> Result<(), Error>
    where
        I: Iterator<Item=&'b String>
    {
        if let Some(field) = attributes
            .find(|field|
                !self.schema.is_primary_key(&field) &&
                !self.schema.has_attribute(&field) &&
                !self.schema.has_foreign_key(&field)
            )
        {
            Err(Error::SchemaValidationFailure {
                schema: self.schema.name.to_string(),
                attribute: field.to_string(),
                message: "Unknown attribute".to_string()
            })
        } else {
            Ok(())
        }
    }

    fn parse_attribute<T: FromStr>(attribute: &str, value: &str) -> Result<T, Error> {
        value.parse::<T>()
            .map_err(|_| Error::InvalidAttribute {
                attribute: attribute.to_string(),
                kind: type_name::<T>().to_string(),
                message: "could not parse value".to_string()
            })
    }

    fn parse_for_field(attribute_type: AttributeType, field: &str, value: &str) -> Result<Attribute, Error> {
        let attribute = match attribute_type {
            AttributeType::Text =>
                Attribute::Text(value.to_string()),
            AttributeType::Integer =>
                Attribute::Integer(Self::parse_attribute(field, value)?),
            AttributeType::Float =>
                Attribute::Float(Self::parse_attribute(field, value)?),
            AttributeType::Boolean =>
                Attribute::Boolean(Self::parse_attribute(field, value)?),
            AttributeType::DateTime =>
                Attribute::DateTime(
                    chrono::DateTime::parse_from_rfc3339(value)
                        .map_err(|err| Error::InvalidAttribute {
                            attribute: field.to_string(),
                            kind: "DateTime".to_string(),
                            message: format!("DateTime string '{}' is invalid. {}", value, err)
                        })?
                        .to_utc()
                )

        };

        Ok(attribute)
    }

    fn fields_for_model(&self) -> impl Iterator<Item=&str> {
        slice::from_ref(&self.schema.primary_key.name)
            .into_iter()
            .chain(
                self.schema.foreign_keys.iter().map(|(name, _)| name)
            )
            .chain(
                self.schema.attributes.iter().map(|(name, _)| name)
            )
            .map(|name| *name)

    }
}

impl<'a> QueryBuilderInterface<'a> for QueryBuilder<'a> {
    fn new(schema: &'a TableSchema) -> Self {
        Self { schema }
    }

    fn query(&self, parameters: &QueryParameters) -> Result<(String, Bindings), Error> {
        let mut query = Vec::new();

        self.build_select_clause(&mut query)?;
        self.build_from_clause(&mut query);
        self.build_join_clause(&parameters.search, &mut query)?;
        let bindings = self.build_where_clause(&parameters.filter, &parameters.search, &mut query)?;
        self.build_order_by_clause(&parameters.sort, &mut query)?;
        self.build_limit_offset_clauses(&parameters.page, &mut query);

        Ok((query.join(" ").to_string(), bindings))
    }

    fn find(&self, id: i32, parameters: &QueryParameters) -> Result<(String, Bindings), Error> {
        let mut query = Vec::new();

        self.build_select_clause(&mut query)?;
        self.build_from_clause(&mut query);
        query.push("WHERE id = ?1".to_string());

        let bindings = Bindings::from([Attribute::Integer(id as i64)]);

        Ok((query.join(" ").to_string(), bindings))
    }

    fn insert(&self, attributes: Attributes, parameters: &QueryParameters) -> Result<(String, Bindings), Error> {
        let mut query = Vec::new();

        let bindings = self.build_insert_clause(attributes, &mut query)?;
        self.build_returning_clause(&parameters.fields, &mut query)?;

        Ok((query.join(" "), bindings))
    }

    fn update(&self, id: i32, attributes: Attributes, parameters: &QueryParameters) -> Result<(String, Bindings), Error> {
        let mut query = Vec::new();
        let bindings = self.build_update_clause(id, attributes, &mut query)?;
        self.build_returning_clause(&parameters.fields, &mut query)?;

        Ok((query.join(" "), bindings))
    }

    fn delete(&self, id: i32) -> (String, Bindings) {
        (
            format!("DELETE FROM {} WHERE id = ?1", self.schema.name),
            Bindings::from([Attribute::Integer(id as i64)])
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::LazyLock;
    use crate::database::schema::{IdentifierType, PrimaryKey};
    use crate::http_wrappers::Uri;

    fn mock_schema(text_index: bool) -> TableSchema<'static> {
        TableSchema {
            name: "my_table",
            primary_key: PrimaryKey {
                name: "id",
                kind: IdentifierType::Integer
            },
            attributes: &[
                ("id", AttributeType::Integer),
                ("col1", AttributeType::Text),
                ("col2", AttributeType::Text),
                ("col3", AttributeType::DateTime),
            ],
            foreign_keys: &[],
            relationships: &[],
            text_index
        }
    }

    static MY_SCHEMA: LazyLock<TableSchema> = LazyLock::new(|| mock_schema(true) );
    static MY_SCHEMA_NO_FTS: LazyLock<TableSchema> = LazyLock::new(|| mock_schema(false));

    fn mock_uri(query: &str) -> Uri {
        format!("http://localhost:8000/resource?{}", query)
            .parse::<Uri>()
            .unwrap()
    }

    fn parse_parameters(query: &str) -> QueryParameters {
        QueryParameters::parse(&mock_uri(query)).unwrap()
    }

    #[test]
    fn test_select_all_fields() {
        let builder = QueryBuilder::new(&MY_SCHEMA);
        let parameters = parse_parameters("");
        let (query, bindings) = builder.query(&parameters).unwrap();

        assert_eq!(query, "SELECT * FROM my_table");
        assert!(bindings.is_empty());
    }

    #[test]
    fn test_select_specific_fields() {
        let builder = QueryBuilder::new(&MY_SCHEMA);
        let parameters = parse_parameters("fields[my_table]=col1,col2");
        let (query, bindings) = builder.query(&parameters).unwrap();

        assert_eq!(query, "SELECT col1, col2 FROM my_table");
        assert!(bindings.is_empty());
    }

    #[test]
    fn test_filter_single_condition() {
        let builder = QueryBuilder::new(&MY_SCHEMA);
        let parameters = parse_parameters("filter[col1]=eq:value1");
        let (query, bindings) = builder.query(&parameters).unwrap();

        assert_eq!(query, "SELECT * FROM my_table WHERE col1 = ?1");
        assert_eq!(bindings, vec![Attribute::Text("value1".to_string())]);
    }

    #[test]
    fn test_filter_multiple_conditions() {
        let builder = QueryBuilder::new(&MY_SCHEMA);
        let parameters = parse_parameters("filter[col1]=eq:value1&filter[col2]=neq:value2");
        let (query, bindings) = builder.query(&parameters).unwrap();

        assert_eq!(query, "SELECT * FROM my_table WHERE col1 = ?1 AND col2 != ?2");
        assert_eq!(bindings, vec![Attribute::Text("value1".to_string()), Attribute::Text("value2".to_string())]);
    }

    #[test]
    fn test_filter_with_like_operator() {
        let builder = QueryBuilder::new(&MY_SCHEMA);
        let parameters = parse_parameters("filter[col1]=like:keyword");
        let (query, bindings) = builder.query(&parameters).unwrap();

        assert_eq!(query, "SELECT * FROM my_table WHERE col1 LIKE ?1");
        assert_eq!(bindings, vec![Attribute::Text("%keyword%".to_string())]);
    }

    #[test]
    fn test_sort_single_field() {
        let builder = QueryBuilder::new(&MY_SCHEMA);
        let parameters = parse_parameters("sort=-col1");
        let (query, bindings) = builder.query(&parameters).unwrap();

        assert_eq!(query, "SELECT * FROM my_table ORDER BY col1 DESC");
        assert!(bindings.is_empty());
    }

    #[test]
    fn test_sort_multiple_fields() {
        let builder = QueryBuilder::new(&MY_SCHEMA);
        let parameters = parse_parameters("sort=-col1,col2");
        let (query, bindings) = builder.query(&parameters).unwrap();

        assert_eq!(query, "SELECT * FROM my_table ORDER BY col1 DESC, col2 ASC");
        assert!(bindings.is_empty());
    }

    #[test]
    fn test_pagination() {
        let builder = QueryBuilder::new(&MY_SCHEMA);
        let parameters = parse_parameters("page[number]=2&page[size]=10");
        let (query, bindings) = builder.query(&parameters).unwrap();

        assert_eq!(query, "SELECT * FROM my_table LIMIT 10 OFFSET 10");
        assert!(bindings.is_empty());
    }

    #[test]
    fn test_complex_query_with_all_features() {
        let builder = QueryBuilder::new(&MY_SCHEMA);
        let parameters = parse_parameters(
            "\
            fields[my_table]=col1,col2&\
            filter[col1]=eq:value1&\
            sort=-col1&\
            page[number]=1&\
            page[size]=5&\
            search=find-me\
            "
        );
        let (query, bindings) = builder.query(&parameters).unwrap();

        assert_eq!(
            query,
            "\
            SELECT col1, col2 FROM my_table \
            JOIN my_table_fts fts ON my_table.id = fts.rowid \
            WHERE my_table_fts MATCH ?1 AND col1 = ?2 \
            ORDER BY col1 DESC \
            LIMIT 5 OFFSET 0\
            "
        );
        assert_eq!(
            bindings,
            vec![Attribute::Text("find-me".to_string()), Attribute::Text("value1".to_string())]
        );
    }

    // Find Tests
    #[test]
    fn test_find_with_all_fields() {
        let builder = QueryBuilder::new(&MY_SCHEMA);
        let parameters = parse_parameters("");
        let (query, bindings) = builder.find(1, &parameters).unwrap();

        assert_eq!(query, "SELECT * FROM my_table WHERE id = ?1");
        assert_eq!(bindings, vec![Attribute::Integer(1)]);
    }

    #[test]
    fn test_find_with_specific_fields() {
        let builder = QueryBuilder::new(&MY_SCHEMA);
        let parameters = parse_parameters("fields[my_table]=col1,col2");
        let (query, bindings) = builder.find(1, &parameters).unwrap();

        assert_eq!(query, "SELECT col1, col2 FROM my_table WHERE id = ?1");
        assert_eq!(bindings, vec![Attribute::Integer(1)]);
    }

    // Insert Tests
    #[test]
    fn test_insert_single_field() {
        let builder = QueryBuilder::new(&MY_SCHEMA);
        let attributes = Attributes::from([
            ("col1".to_string(), Attribute::Text("value1".to_string())),
        ]);
        let parameters = parse_parameters("");
        let (query, bindings) = builder.insert(attributes, &parameters).unwrap();

        assert_eq!(query, "INSERT INTO my_table(col1) VALUES (?1) RETURNING *");
        assert_eq!(bindings, vec![Attribute::Text("value1".to_string())]);
    }

    #[test]
    fn test_insert_multiple_fields() {
        let builder = QueryBuilder::new(&MY_SCHEMA);
        let attributes = Attributes::from([
            ("col1".to_string(), Attribute::Text("value1".to_string())),
            ("col2".to_string(), Attribute::Integer(42))
        ]);
        let parameters = parse_parameters("");
        let (query, bindings) = builder.insert(attributes, &parameters).unwrap();

        assert_eq!(query, "INSERT INTO my_table(col1, col2) VALUES (?1, ?2) RETURNING *");
        assert_eq!(bindings, vec![Attribute::Text("value1".to_string()), Attribute::Integer(42)]);
    }

    #[test]
    fn test_insert_with_returning_fields() {
        let builder = QueryBuilder::new(&MY_SCHEMA);
        let attributes = Attributes::from([
            ("col1".to_string(), Attribute::Text("value1".to_string())),
        ]);
        let parameters = parse_parameters("fields[my_table]=id,col1");
        let (query, bindings) = builder.insert(attributes, &parameters).unwrap();

        assert_eq!(query, "INSERT INTO my_table(col1) VALUES (?1) RETURNING id, col1");
        assert_eq!(bindings, vec![Attribute::Text("value1".to_string())]);
    }

    #[test]
    fn test_insert_with_empty_attributes() {
        let builder = QueryBuilder::new(&MY_SCHEMA);
        let attributes = Attributes::new();
        let parameters = parse_parameters("");
        let (query, bindings) = builder.insert(attributes, &parameters).unwrap();

        assert_eq!(query, "INSERT INTO my_table() VALUES () RETURNING *");
        assert!(bindings.is_empty());
    }

    // Update Tests
    #[test]
    fn test_update_single_field() {
        let builder = QueryBuilder::new(&MY_SCHEMA);
        let attributes = Attributes::from([
            ("col1".to_string(), Attribute::Text("new_value".to_string())),
        ]);
        let parameters = parse_parameters("");
        let (query, bindings) = builder.update(1, attributes, &parameters).unwrap();

        assert_eq!(query, "UPDATE my_table SET col1 = ?1 WHERE id = ?2 RETURNING *");
        assert_eq!(bindings, vec![Attribute::Text("new_value".to_string()), Attribute::Integer(1)]);
    }

    #[test]
    fn test_update_multiple_fields() {
        let builder = QueryBuilder::new(&MY_SCHEMA);
        let attributes = Attributes::from([
            ("col1".to_string(), Attribute::Text("new_value".to_string())),
            ("col2".to_string(), Attribute::Integer(42))
        ]);
        let parameters = parse_parameters("");
        let (query, bindings) = builder.update(1, attributes, &parameters).unwrap();

        assert_eq!(query, "UPDATE my_table SET col1 = ?1, col2 = ?2 WHERE id = ?3 RETURNING *");
        assert_eq!(
            bindings,
            vec![Attribute::Text("new_value".to_string()), Attribute::Integer(42), Attribute::Integer(1)]
        );
    }

    #[test]
    fn test_update_with_returning_fields() {
        let builder = QueryBuilder::new(&MY_SCHEMA);
        let attributes = Attributes::from([
            ("col1".to_string(), Attribute::Text("new_value".to_string())),
        ]);
        let parameters = parse_parameters("fields[my_table]=id,col1");
        let (query, bindings) = builder.update(1, attributes, &parameters).unwrap();

        assert_eq!(query, "UPDATE my_table SET col1 = ?1 WHERE id = ?2 RETURNING id, col1");
        assert_eq!(bindings, vec![Attribute::Text("new_value".to_string()), Attribute::Integer(1)]);
    }

    #[test]
    fn test_update_with_empty_attributes() {
        let builder = QueryBuilder::new(&MY_SCHEMA);
        let attributes = Attributes::new();
        let parameters = parse_parameters("");
        let (query, bindings) = builder.update(1, attributes, &parameters).unwrap();

        assert_eq!(query, "UPDATE my_table WHERE id = ?1 RETURNING *");
        assert_eq!(bindings, vec![Attribute::Integer(1)]);
    }

    // Delete Tests
    #[test]
    fn test_delete() {
        let builder = QueryBuilder::new(&MY_SCHEMA);
        let (query, bindings) = builder.delete(1);

        assert_eq!(query, "DELETE FROM my_table WHERE id = ?1");
        assert_eq!(bindings, vec![Attribute::Integer(1)]);
    }

    // ExtractedAttributes Tests
    #[test]
    fn test_extracted_attributes_placeholders() {
        let extracted = ExtractedAttributes {
            fields: vec!["col1".to_string(), "col2".to_string()],
            values: vec![Attribute::Text("value1".to_string()), Attribute::Integer(42)],
        };

        let placeholders = extracted.to_placeholders();
        assert_eq!(placeholders, vec!["?1", "?2"]);
    }

    #[test]
    fn test_placeholders_with_empty_fields() {
        let extracted = ExtractedAttributes {
            fields: vec![],
            values: vec![],
        };

        let placeholders = extracted.to_placeholders();
        assert!(placeholders.is_empty());
    }

    // Additional Tests for Untested Functionality
    #[test]
    fn test_filter_with_invalid_attribute() {
        let builder = QueryBuilder::new(&MY_SCHEMA);
        let parameters = parse_parameters("filter[invalid_col]=eq:value1");
        let result = builder.query(&parameters);

        assert!(result.is_err());
    }

    #[test]
    fn test_insert_with_invalid_attribute() {
        let builder = QueryBuilder::new(&MY_SCHEMA);
        let attributes = Attributes::from([
            ("invalid_col".to_string(), Attribute::Text("value1".to_string())),
        ]);
        let parameters = parse_parameters("");
        let result = builder.insert(attributes, &parameters);

        assert!(result.is_err());
    }

    #[test]
    fn test_update_with_invalid_attribute() {
        let builder = QueryBuilder::new(&MY_SCHEMA);
        let attributes = Attributes::from([
            ("invalid_col".to_string(), Attribute::Text("new_value".to_string())),
        ]);
        let parameters = parse_parameters("");
        let result = builder.update(1, attributes, &parameters);

        assert!(result.is_err());
    }

    #[test]
    fn test_filter_with_like_operator_on_non_text_attribute() {
        let builder = QueryBuilder::new(&MY_SCHEMA);
        let parameters = parse_parameters("filter[id]=like:1");
        let result = builder.query(&parameters);

        assert!(result.is_err());
    }

    #[test]
    fn test_filter_with_invalid_value_for_attribute_type() {
        let builder = QueryBuilder::new(&MY_SCHEMA);
        let parameters = parse_parameters("filter[id]=eq:not_a_number");
        let result = builder.query(&parameters);

        assert!(result.is_err());
    }

    #[test]
    fn test_filter_with_invalid_date_time_value() {
        let builder = QueryBuilder::new(&MY_SCHEMA);
        let parameters = parse_parameters("filter[col3]=eq:invalid_date");
        let result = builder.query(&parameters);

        assert!(result.is_err());
    }

    #[test]
    fn test_sort_with_invalid_attribute() {
        let builder = QueryBuilder::new(&MY_SCHEMA);
        let parameters = parse_parameters("sort=-invalid_col");
        let result = builder.query(&parameters);

        assert!(result.is_err());
    }

    #[test]
    fn test_insert_with_invalid_returning_fields() {
        let builder = QueryBuilder::new(&MY_SCHEMA);
        let attributes = Attributes::from([
            ("col1".to_string(), Attribute::Text("value1".to_string())),
        ]);
        let parameters = parse_parameters("fields[my_table]=invalid_col");
        let result = builder.insert(attributes, &parameters);

        assert!(result.is_err());
    }

    #[test]
    fn test_update_with_invalid_returning_fields() {
        let builder = QueryBuilder::new(&MY_SCHEMA);
        let attributes = Attributes::from([
            ("col1".to_string(), Attribute::Text("new_value".to_string())),
        ]);
        let parameters = parse_parameters("fields[my_table]=invalid_col");
        let result = builder.update(1, attributes, &parameters);

        assert!(result.is_err());
    }

    #[test]
    fn test_search_with_single_term() {
        let builder = QueryBuilder::new(&MY_SCHEMA);
        let parameters = parse_parameters("search=a-value-to-search");

        let (query, bindings) = builder.query(&parameters).unwrap();

        assert_eq!(
            query,
            "\
            SELECT * FROM my_table \
            JOIN my_table_fts fts ON my_table.id = fts.rowid \
            WHERE my_table_fts MATCH ?1\
            ".to_string()
        );
        assert_eq!(bindings, vec![Attribute::Text("a-value-to-search".to_string())]);
    }

    #[test]
    fn test_search_with_multiple_terms() {
        let builder = QueryBuilder::new(&MY_SCHEMA);
        let parameters = parse_parameters("search=a-value,another-value");

        let (query, bindings) = builder.query(&parameters).unwrap();

        assert_eq!(
            query,
            "\
            SELECT * FROM my_table \
            JOIN my_table_fts fts ON my_table.id = fts.rowid \
            WHERE my_table_fts MATCH ?1 AND my_table_fts MATCH ?2\
            ".to_string()
        );
        assert_eq!(
            bindings,
            vec![Attribute::Text("a-value".to_string()), Attribute::Text("another-value".to_string())]
        );
    }

    #[test]
    fn test_search_on_table_without_text_index() {
        let builder = QueryBuilder::new(&MY_SCHEMA_NO_FTS);
        let parameters = parse_parameters("search=a-value-to-search");

        let result = builder.query(&parameters);

        assert!(result.is_err());
    }
}