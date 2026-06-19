use crate::database::attributes::Identifier;
use crate::database::{
    attributes::{Attribute, Attributes},
    error::Error,
    query_builder::QueryBuilder as QueryBuilderInterface,
    query_parameters::{
        FieldsParameters, FilterParameters, FilterValue, PageParameters, QueryParameters,
        SearchParameters, SortDirection, SortParameters, SortingAttribute,
    },
    schema::{AttributeType, TableSchema},
};
use itertools::Itertools;

struct ExtractedAttributes {
    fields: Vec<String>,
    values: Vec<Attribute>,
}

impl ExtractedAttributes {
    pub(super) fn to_placeholders(&self) -> Vec<String> {
        (1..=self.fields.len()).map(|i| format!("?{i}")).collect()
    }
}

pub type Bindings = Vec<Attribute>;

pub struct QueryBuilder<'sch> {
    schema: &'sch TableSchema<'sch>,
}

impl<'sch> QueryBuilder<'sch> {
    fn build_select_clause(&self, fields: &FieldsParameters, query: &mut Vec<String>) {
        query.extend(["SELECT".to_string(), self.fields_for_model(fields, true)]);
    }

    fn build_insert_clause(&self, attributes: Attributes, query: &mut Vec<String>) -> Bindings {
        let attributes = self.extract_attributes(attributes);
        let placeholders = attributes.to_placeholders();

        query.extend([
            "INSERT INTO".to_string(),
            format!("{}({})", self.schema.name, attributes.fields.join(", ")),
            format!("VALUES ({})", placeholders.join(", ")),
        ]);

        attributes.values
    }

    fn build_update_clause(
        &self,
        id: Identifier,
        attributes: Attributes,
        query: &mut Vec<String>,
    ) -> Bindings {
        let attributes = self.extract_attributes(attributes);
        let placeholders = attributes.to_placeholders();
        query.extend(["UPDATE".to_string(), self.schema.name.to_string()]);

        if !attributes.fields.is_empty() {
            let fields = attributes
                .fields
                .into_iter()
                .zip(placeholders)
                .map(|(field, placeholder)| format!("{} = {}", field, placeholder))
                .join(", ");
            query.extend(["SET".to_string(), fields]);
        }

        query.push(format!("WHERE id = ?{}", attributes.values.len() + 1));
        [attributes.values.as_slice(), &[Attribute::from(id)]].concat()
    }

    fn build_from_clause(&self, query: &mut Vec<String>) {
        query.extend(["FROM".to_string(), self.schema.name.to_string()]);
    }

    fn build_join_clause(
        &self,
        search: &Option<SearchParameters>,
        query: &mut Vec<String>,
    ) -> Result<(), Error> {
        if search.is_none() {
            return Ok(());
        }

        if !self.schema.text_index {
            return Err(Error::InvalidOperation {
                schema: self.schema.name.to_string(),
                operation: "MATCH".to_string(),
                message: "table does not support full-text search".to_string(),
            });
        }

        query.push(format!(
            "JOIN {}_fts fts ON {}.id = fts.rowid",
            self.schema.name, self.schema.name
        ));

        Ok(())
    }

    fn build_where_clause(
        &self,
        filter: &Option<FilterParameters>,
        search: &Option<SearchParameters>,
        query: &mut Vec<String>,
    ) -> Result<Bindings, Error> {
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
                bindings.push(Attribute::Text(value.to_string()));
                i += 1;
            }
        }

        if let Some(filter) = filter {
            for (field, filters) in filter {
                let table = self.schema.name;
                let kind = self.schema.attribute_type(field)?;

                for filter in filters {
                    match filter {
                        filter @ In(_) | filter @ NotIn(_) => {
                            let (operator, values) = match filter {
                                In(values) => ("IN", values),
                                NotIn(values) => ("NOT IN", values),
                                _ => unreachable!(),
                            };

                            let placeholders = values
                                .iter()
                                .enumerate()
                                .map(|(pos, _)| format!("?{}", i + pos))
                                .join(",");
                            filter_query
                                .push(format!("{table}.{field} {operator} ({placeholders})",));

                            bindings.extend(values.clone());
                            i += values.len();
                        }
                        Like(value) => {
                            let binding = if matches!(kind, AttributeType::Text) {
                                Attribute::Text(format!("%{}%", value))
                            } else {
                                return Err(Error::SchemaValidationFailure {
                                    schema: self.schema.name.to_string(),
                                    attribute: field.to_string(),
                                    message: "'LIKE' operator cannot be applied to attribute"
                                        .to_string(),
                                });
                            };
                            filter_query.push(format!("{table}.{field} LIKE ?{i}"));
                            bindings.push(binding);
                            i += 1;
                        }
                        filter => {
                            let (operator, binding) = match filter {
                                Equal(value) => ("=", value),
                                NotEqual(value) => ("!=", value),
                                GreaterThan(value) => (">", value),
                                GreaterThanOrEqual(value) => (">=", value),
                                LessThan(value) => ("<", value),
                                LessThanOrEqual(value) => ("<=", value),
                                _ => unreachable!(),
                            };

                            filter_query.push(format!("{table}.{field} {operator} ?{i}"));
                            bindings.push(binding.clone());
                            i += 1;
                        }
                    }
                }
            }
        }

        query.push(filter_query.join(" AND ").to_string());
        Ok(bindings)
    }

    fn build_order_by_clause(&self, sort: &Option<SortParameters>, query: &mut Vec<String>) {
        if let Some(fields) = sort {
            query.push("ORDER BY".to_string());
            let mut sort_query = Vec::new();

            for SortingAttribute {
                attribute: field,
                direction,
            } in fields
            {
                let direction = match direction {
                    SortDirection::Ascending => "ASC",
                    SortDirection::Descending => "DESC",
                };
                sort_query.push(format!("{}.{} {}", self.schema.name, field, direction));
            }

            query.push(sort_query.join(", ").to_string());
        }
    }

    fn build_limit_offset_clauses(&self, page: &Option<PageParameters>, query: &mut Vec<String>) {
        if let Some(PageParameters { number, size }) = page {
            let limit = size.to_string();
            let offset = ((number - 1) * size).to_string();

            query.extend(["LIMIT".to_string(), limit, "OFFSET".to_string(), offset]);
        }
    }

    fn build_returning_clause(&self, fields: &FieldsParameters, query: &mut Vec<String>) {
        query.extend([
            "RETURNING".to_string(),
            self.fields_for_model(fields, false),
        ]);
    }

    /// Renders the comma-separated column list for the given model, primary key first. When
    /// `qualified` is set, each column is prefixed with the table name.
    fn fields_for_model(&self, fields: &FieldsParameters, qualified: bool) -> String {
        let fields = fields
            .get(self.schema.name)
            .expect("Columns for all requested models should have been pre-loaded by the query parameters parser")
            .iter()
            .map(|field| if self.schema.has_attribute(field) || self.schema.has_foreign_key(field) {
                field
            } else {
                self.schema
                    .relationship(field)
                    .expect(
                        "\
                        All columns provided to the query builder should have been pre-validated by \
                        the query parameters parser\
                        "
                    )
                    .related_resource()
                    .keys.own
            });

        let columns = || [self.schema.primary_key.name].into_iter().chain(fields);

        if qualified {
            columns()
                .map(|column| format!("{}.{}", self.schema.name, column))
                .join(", ")
        } else {
            columns().join(", ")
        }
    }

    fn extract_attributes(&self, attributes: Attributes) -> ExtractedAttributes {
        let mut fields = Vec::<String>::new();
        let mut values = Vec::<Attribute>::new();

        for (field, value) in attributes {
            fields.push(field);
            values.push(value);
        }

        ExtractedAttributes { fields, values }
    }
}

impl<'sch> QueryBuilderInterface<'sch> for QueryBuilder<'sch> {
    fn new(schema: &'sch TableSchema) -> Self {
        Self { schema }
    }

    fn query(&self, parameters: &QueryParameters) -> Result<(String, Bindings), Error> {
        let mut query = Vec::new();

        self.build_select_clause(&parameters.fields, &mut query);
        self.build_from_clause(&mut query);
        self.build_join_clause(&parameters.search, &mut query)?;
        let bindings =
            self.build_where_clause(&parameters.filter, &parameters.search, &mut query)?;
        self.build_order_by_clause(&parameters.sort, &mut query);
        self.build_limit_offset_clauses(&parameters.page, &mut query);

        Ok((query.join(" ").to_string(), bindings))
    }

    fn find(
        &self,
        id: Identifier,
        parameters: &QueryParameters,
    ) -> Result<(String, Bindings), Error> {
        let mut query = Vec::new();

        self.build_select_clause(&parameters.fields, &mut query);
        self.build_from_clause(&mut query);
        query.push("WHERE id = ?1".to_string());

        let bindings = Bindings::from([Attribute::from(id)]);

        Ok((query.join(" ").to_string(), bindings))
    }

    fn insert(
        &self,
        attributes: Attributes,
        parameters: &QueryParameters,
    ) -> Result<(String, Bindings), Error> {
        let mut query = Vec::new();

        let bindings = self.build_insert_clause(attributes, &mut query);
        self.build_returning_clause(&parameters.fields, &mut query);

        Ok((query.join(" "), bindings))
    }

    fn update(
        &self,
        id: Identifier,
        attributes: Attributes,
        parameters: &QueryParameters,
    ) -> Result<(String, Bindings), Error> {
        let mut query = Vec::new();
        let bindings = self.build_update_clause(id, attributes, &mut query);
        self.build_returning_clause(&parameters.fields, &mut query);

        Ok((query.join(" "), bindings))
    }

    fn delete(&self, id: Identifier) -> (String, Bindings) {
        (
            format!("DELETE FROM {} WHERE id = ?1", self.schema.name),
            [Attribute::from(id)].into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::adapters::SqliteAdapter;
    use crate::database::registry::Registry as DatabaseRegistry;
    use crate::database::schema::{IdentifierType, PrimaryKey};
    use crate::http_wrappers::Uri;
    use rusqlite::Connection;

    type Registry = DatabaseRegistry<'static, SqliteAdapter>;

    static MY_SCHEMA: TableSchema = my_schema(true);
    static MY_SCHEMA_NO_FTS: TableSchema = my_schema(false);

    const fn my_schema(text_index: bool) -> TableSchema<'static> {
        TableSchema {
            name: "my_table",
            primary_key: PrimaryKey {
                name: "id",
                kind: IdentifierType::Integer,
            },
            attributes: &[
                ("col1", AttributeType::Text),
                ("col2", AttributeType::Text),
                ("col3", AttributeType::Integer),
            ],
            foreign_keys: &[],
            relationships: &[],
            text_index,
        }
    }

    static FTS_SCHEMAS: [&TableSchema; 1] = [&MY_SCHEMA];
    static PLAIN_SCHEMAS: [&TableSchema; 1] = [&MY_SCHEMA_NO_FTS];

    fn registry(schemas: &'static [&'static TableSchema]) -> Registry {
        DatabaseRegistry::try_new(Connection::open_in_memory().unwrap(), schemas).unwrap()
    }

    fn mock_uri(query: &str) -> Uri {
        format!("http://localhost:8000/my_table?{}", query)
            .parse::<Uri>()
            .unwrap()
    }

    fn parse<'req>(registry: &Registry, uri: &'req Uri) -> QueryParameters<'static, 'req> {
        QueryParameters::parse(uri, &MY_SCHEMA, registry).unwrap()
    }

    #[test]
    fn test_select_all_fields() {
        let registry = registry(&FTS_SCHEMAS);
        let uri = mock_uri("");
        let (query, bindings) = QueryBuilder::new(&MY_SCHEMA)
            .query(&parse(&registry, &uri))
            .unwrap();

        assert_eq!(
            query,
            "SELECT my_table.id, my_table.col1, my_table.col2, my_table.col3 FROM my_table"
        );
        assert!(bindings.is_empty());
    }

    #[test]
    fn test_select_specific_fields() {
        let registry = registry(&FTS_SCHEMAS);
        let uri = mock_uri("fields[my_table]=col1,col2");
        let (query, bindings) = QueryBuilder::new(&MY_SCHEMA)
            .query(&parse(&registry, &uri))
            .unwrap();

        assert_eq!(
            query,
            "SELECT my_table.id, my_table.col1, my_table.col2 FROM my_table"
        );
        assert!(bindings.is_empty());
    }

    #[test]
    fn test_filter_single_condition() {
        let registry = registry(&FTS_SCHEMAS);
        let uri = mock_uri("filter[col1]=eq:value1");
        let (query, bindings) = QueryBuilder::new(&MY_SCHEMA)
            .query(&parse(&registry, &uri))
            .unwrap();

        assert_eq!(
            query,
            "SELECT my_table.id, my_table.col1, my_table.col2, my_table.col3 FROM my_table WHERE my_table.col1 = ?1"
        );
        assert_eq!(bindings, vec![Attribute::Text("value1".to_string())]);
    }

    #[test]
    fn test_filter_multiple_conditions() {
        let registry = registry(&FTS_SCHEMAS);
        let uri = mock_uri("filter[col1]=eq:value1&filter[col2]=neq:value2");
        let (query, bindings) = QueryBuilder::new(&MY_SCHEMA)
            .query(&parse(&registry, &uri))
            .unwrap();

        assert_eq!(
            query,
            "SELECT my_table.id, my_table.col1, my_table.col2, my_table.col3 FROM my_table WHERE my_table.col1 = ?1 AND my_table.col2 != ?2"
        );
        assert_eq!(
            bindings,
            vec![
                Attribute::Text("value1".to_string()),
                Attribute::Text("value2".to_string())
            ]
        );
    }

    #[test]
    fn test_filter_with_like_operator() {
        let registry = registry(&FTS_SCHEMAS);
        let uri = mock_uri("filter[col1]=like:keyword");
        let (query, bindings) = QueryBuilder::new(&MY_SCHEMA)
            .query(&parse(&registry, &uri))
            .unwrap();

        assert_eq!(
            query,
            "SELECT my_table.id, my_table.col1, my_table.col2, my_table.col3 FROM my_table WHERE my_table.col1 LIKE ?1"
        );
        assert_eq!(bindings, vec![Attribute::Text("%keyword%".to_string())]);
    }

    #[test]
    fn test_sort_single_field() {
        let registry = registry(&FTS_SCHEMAS);
        let uri = mock_uri("sort=-col1");
        let (query, bindings) = QueryBuilder::new(&MY_SCHEMA)
            .query(&parse(&registry, &uri))
            .unwrap();

        assert_eq!(
            query,
            "SELECT my_table.id, my_table.col1, my_table.col2, my_table.col3 FROM my_table ORDER BY my_table.col1 DESC"
        );
        assert!(bindings.is_empty());
    }

    #[test]
    fn test_sort_multiple_fields() {
        let registry = registry(&FTS_SCHEMAS);
        let uri = mock_uri("sort=-col1,col2");
        let (query, bindings) = QueryBuilder::new(&MY_SCHEMA)
            .query(&parse(&registry, &uri))
            .unwrap();

        assert_eq!(
            query,
            "SELECT my_table.id, my_table.col1, my_table.col2, my_table.col3 FROM my_table ORDER BY my_table.col1 DESC, my_table.col2 ASC"
        );
        assert!(bindings.is_empty());
    }

    #[test]
    fn test_pagination() {
        let registry = registry(&FTS_SCHEMAS);
        let uri = mock_uri("page[number]=2&page[size]=10");
        let (query, bindings) = QueryBuilder::new(&MY_SCHEMA)
            .query(&parse(&registry, &uri))
            .unwrap();

        assert_eq!(
            query,
            "SELECT my_table.id, my_table.col1, my_table.col2, my_table.col3 FROM my_table LIMIT 10 OFFSET 10"
        );
        assert!(bindings.is_empty());
    }

    #[test]
    fn test_complex_query_with_all_features() {
        let registry = registry(&FTS_SCHEMAS);
        let uri = mock_uri(
            "\
            fields[my_table]=col1,col2&\
            filter[col1]=eq:value1&\
            sort=-col1&\
            page[number]=1&\
            page[size]=5&\
            search=find-me\
            ",
        );
        let (query, bindings) = QueryBuilder::new(&MY_SCHEMA)
            .query(&parse(&registry, &uri))
            .unwrap();

        assert_eq!(
            query,
            "\
            SELECT my_table.id, my_table.col1, my_table.col2 FROM my_table \
            JOIN my_table_fts fts ON my_table.id = fts.rowid \
            WHERE my_table_fts MATCH ?1 AND my_table.col1 = ?2 \
            ORDER BY my_table.col1 DESC \
            LIMIT 5 OFFSET 0\
            "
        );
        assert_eq!(
            bindings,
            vec![
                Attribute::Text("find-me".to_string()),
                Attribute::Text("value1".to_string())
            ]
        );
    }

    #[test]
    fn test_find_with_all_fields() {
        let registry = registry(&FTS_SCHEMAS);
        let uri = mock_uri("");
        let (query, bindings) = QueryBuilder::new(&MY_SCHEMA)
            .find(Identifier::Integer(1), &parse(&registry, &uri))
            .unwrap();

        assert_eq!(
            query,
            "SELECT my_table.id, my_table.col1, my_table.col2, my_table.col3 FROM my_table WHERE id = ?1"
        );
        assert_eq!(bindings, vec![Attribute::Integer(1)]);
    }

    #[test]
    fn test_find_with_specific_fields() {
        let registry = registry(&FTS_SCHEMAS);
        let uri = mock_uri("fields[my_table]=col1,col2");
        let (query, bindings) = QueryBuilder::new(&MY_SCHEMA)
            .find(Identifier::Integer(1), &parse(&registry, &uri))
            .unwrap();

        assert_eq!(
            query,
            "SELECT my_table.id, my_table.col1, my_table.col2 FROM my_table WHERE id = ?1"
        );
        assert_eq!(bindings, vec![Attribute::Integer(1)]);
    }

    #[test]
    fn test_insert_single_field() {
        let registry = registry(&FTS_SCHEMAS);
        let uri = mock_uri("");
        let attributes =
            Attributes::from_iter([("col1".to_string(), Attribute::Text("value1".to_string()))]);
        let (query, bindings) = QueryBuilder::new(&MY_SCHEMA)
            .insert(attributes, &parse(&registry, &uri))
            .unwrap();

        assert_eq!(
            query,
            "INSERT INTO my_table(col1) VALUES (?1) RETURNING id, col1, col2, col3"
        );
        assert_eq!(bindings, vec![Attribute::Text("value1".to_string())]);
    }

    #[test]
    fn test_insert_multiple_fields() {
        let registry = registry(&FTS_SCHEMAS);
        let uri = mock_uri("");
        let attributes = Attributes::from_iter([
            ("col1".to_string(), Attribute::Text("value1".to_string())),
            ("col2".to_string(), Attribute::Integer(42)),
        ]);
        let (query, bindings) = QueryBuilder::new(&MY_SCHEMA)
            .insert(attributes, &parse(&registry, &uri))
            .unwrap();

        assert_eq!(
            query,
            "INSERT INTO my_table(col1, col2) VALUES (?1, ?2) RETURNING id, col1, col2, col3"
        );
        assert_eq!(
            bindings,
            vec![
                Attribute::Text("value1".to_string()),
                Attribute::Integer(42)
            ]
        );
    }

    #[test]
    fn test_insert_with_returning_fields() {
        let registry = registry(&FTS_SCHEMAS);
        let uri = mock_uri("fields[my_table]=col1");
        let attributes =
            Attributes::from_iter([("col1".to_string(), Attribute::Text("value1".to_string()))]);
        let (query, bindings) = QueryBuilder::new(&MY_SCHEMA)
            .insert(attributes, &parse(&registry, &uri))
            .unwrap();

        assert_eq!(
            query,
            "INSERT INTO my_table(col1) VALUES (?1) RETURNING id, col1"
        );
        assert_eq!(bindings, vec![Attribute::Text("value1".to_string())]);
    }

    #[test]
    fn test_insert_with_empty_attributes() {
        let registry = registry(&FTS_SCHEMAS);
        let uri = mock_uri("");
        let (query, bindings) = QueryBuilder::new(&MY_SCHEMA)
            .insert(Attributes::new(), &parse(&registry, &uri))
            .unwrap();

        assert_eq!(
            query,
            "INSERT INTO my_table() VALUES () RETURNING id, col1, col2, col3"
        );
        assert!(bindings.is_empty());
    }

    #[test]
    fn test_update_single_field() {
        let registry = registry(&FTS_SCHEMAS);
        let uri = mock_uri("");
        let attributes =
            Attributes::from_iter([("col1".to_string(), Attribute::Text("new_value".to_string()))]);
        let (query, bindings) = QueryBuilder::new(&MY_SCHEMA)
            .update(Identifier::Integer(1), attributes, &parse(&registry, &uri))
            .unwrap();

        assert_eq!(
            query,
            "UPDATE my_table SET col1 = ?1 WHERE id = ?2 RETURNING id, col1, col2, col3"
        );
        assert_eq!(
            bindings,
            vec![
                Attribute::Text("new_value".to_string()),
                Attribute::Integer(1)
            ]
        );
    }

    #[test]
    fn test_update_multiple_fields() {
        let registry = registry(&FTS_SCHEMAS);
        let uri = mock_uri("");
        let attributes = Attributes::from_iter([
            ("col1".to_string(), Attribute::Text("new_value".to_string())),
            ("col2".to_string(), Attribute::Integer(42)),
        ]);
        let (query, bindings) = QueryBuilder::new(&MY_SCHEMA)
            .update(Identifier::Integer(1), attributes, &parse(&registry, &uri))
            .unwrap();

        assert_eq!(
            query,
            "UPDATE my_table SET col1 = ?1, col2 = ?2 WHERE id = ?3 RETURNING id, col1, col2, col3"
        );
        assert_eq!(
            bindings,
            vec![
                Attribute::Text("new_value".to_string()),
                Attribute::Integer(42),
                Attribute::Integer(1)
            ]
        );
    }

    #[test]
    fn test_update_with_returning_fields() {
        let registry = registry(&FTS_SCHEMAS);
        let uri = mock_uri("fields[my_table]=col1");
        let attributes =
            Attributes::from_iter([("col1".to_string(), Attribute::Text("new_value".to_string()))]);
        let (query, bindings) = QueryBuilder::new(&MY_SCHEMA)
            .update(Identifier::Integer(1), attributes, &parse(&registry, &uri))
            .unwrap();

        assert_eq!(
            query,
            "UPDATE my_table SET col1 = ?1 WHERE id = ?2 RETURNING id, col1"
        );
        assert_eq!(
            bindings,
            vec![
                Attribute::Text("new_value".to_string()),
                Attribute::Integer(1)
            ]
        );
    }

    #[test]
    fn test_update_with_empty_attributes() {
        let registry = registry(&FTS_SCHEMAS);
        let uri = mock_uri("");
        let (query, bindings) = QueryBuilder::new(&MY_SCHEMA)
            .update(
                Identifier::Integer(1),
                Attributes::new(),
                &parse(&registry, &uri),
            )
            .unwrap();

        assert_eq!(
            query,
            "UPDATE my_table WHERE id = ?1 RETURNING id, col1, col2, col3"
        );
        assert_eq!(bindings, vec![Attribute::Integer(1)]);
    }

    #[test]
    fn test_delete() {
        let (query, bindings) = QueryBuilder::new(&MY_SCHEMA).delete(Identifier::Integer(1));

        assert_eq!(query, "DELETE FROM my_table WHERE id = ?1");
        assert_eq!(bindings, vec![Attribute::Integer(1)]);
    }

    #[test]
    fn test_extracted_attributes_placeholders() {
        let extracted = ExtractedAttributes {
            fields: vec!["col1".to_string(), "col2".to_string()],
            values: vec![
                Attribute::Text("value1".to_string()),
                Attribute::Integer(42),
            ],
        };

        assert_eq!(extracted.to_placeholders(), vec!["?1", "?2"]);
    }

    #[test]
    fn test_placeholders_with_empty_fields() {
        let extracted = ExtractedAttributes {
            fields: vec![],
            values: vec![],
        };

        assert!(extracted.to_placeholders().is_empty());
    }

    #[test]
    fn test_filter_with_like_operator_on_non_text_attribute() {
        let registry = registry(&FTS_SCHEMAS);
        let uri = mock_uri("filter[col3]=like:1");
        let result = QueryBuilder::new(&MY_SCHEMA).query(&parse(&registry, &uri));

        assert!(result.is_err());
    }

    #[test]
    fn test_search_with_single_term() {
        let registry = registry(&FTS_SCHEMAS);
        let uri = mock_uri("search=a-value-to-search");
        let (query, bindings) = QueryBuilder::new(&MY_SCHEMA)
            .query(&parse(&registry, &uri))
            .unwrap();

        assert_eq!(
            query,
            "\
            SELECT my_table.id, my_table.col1, my_table.col2, my_table.col3 FROM my_table \
            JOIN my_table_fts fts ON my_table.id = fts.rowid \
            WHERE my_table_fts MATCH ?1\
            "
        );
        assert_eq!(
            bindings,
            vec![Attribute::Text("a-value-to-search".to_string())]
        );
    }

    #[test]
    fn test_search_with_multiple_terms() {
        let registry = registry(&FTS_SCHEMAS);
        let uri = mock_uri("search=a-value,another-value");
        let (query, bindings) = QueryBuilder::new(&MY_SCHEMA)
            .query(&parse(&registry, &uri))
            .unwrap();

        assert_eq!(
            query,
            "\
            SELECT my_table.id, my_table.col1, my_table.col2, my_table.col3 FROM my_table \
            JOIN my_table_fts fts ON my_table.id = fts.rowid \
            WHERE my_table_fts MATCH ?1 AND my_table_fts MATCH ?2\
            "
        );
        assert_eq!(
            bindings,
            vec![
                Attribute::Text("a-value".to_string()),
                Attribute::Text("another-value".to_string())
            ]
        );
    }

    #[test]
    fn test_search_on_table_without_text_index() {
        let registry = registry(&PLAIN_SCHEMAS);
        let uri = mock_uri("search=a-value-to-search");
        let parameters = QueryParameters::parse(&uri, &MY_SCHEMA_NO_FTS, &registry).unwrap();
        let result = QueryBuilder::new(&MY_SCHEMA_NO_FTS).query(&parameters);

        assert!(result.is_err());
    }
}
