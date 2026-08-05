use super::*;
use crate::database::registry::Registry as DatabaseRegistry;
use crate::database::schema::SchemaBuilder;
use crate::http_wrappers::Uri;
use indexmap::IndexSet;
use std::error::Error as StdError;

type Registry = DatabaseRegistry<'static>;

fn my_schema(text_index: bool) -> SchemaBuilder<'static> {
    let builder = SchemaBuilder::table("my_table")
        .attribute("col1", AttributeType::Text)
        .attribute("col2", AttributeType::Text)
        .attribute("col3", AttributeType::Integer);

    if text_index {
        builder.text_index()
    } else {
        builder
    }
}

fn registry(text_index: bool) -> Registry {
    DatabaseRegistry::try_new([my_schema(text_index)]).expect("schema set is consistent")
}

fn schema(registry: &Registry) -> &Schema<'_> {
    registry.schema("my_table").expect("my_table is registered")
}

fn mock_uri(query: &str) -> Uri {
    format!("http://localhost:8000/my_table?{}", query)
        .parse::<Uri>()
        .expect("mock uri is valid")
}

fn parse<'sch, 'req>(registry: &'sch Registry, uri: &'req Uri) -> QueryParameters<'sch, 'req> {
    QueryParameters::parse(uri, schema(registry), registry).expect("query parses")
}

fn scoped_to_id<'sch, 'req>(
    registry: &'sch Registry,
    uri: &'req Uri,
    id: i64,
) -> QueryParameters<'sch, 'req> {
    let mut parameters =
        QueryParameters::parse(uri, schema(registry), registry).expect("query parses");
    parameters.filter = Some(FilterParameters::from([(
        "id",
        vec![FilterValue::Equal(Attribute::Integer(id))],
    )]));
    parameters
}

#[test]
fn test_select_all_fields() -> Result<(), Box<dyn StdError>> {
    let registry = registry(true);
    let uri = mock_uri("");
    let (query, bindings) = QueryBuilder::new(schema(&registry))
        .query(&parse(&registry, &uri))?
        .ok_or("query should be satisfiable")?;

    assert_eq!(
        query,
        "SELECT my_table.id, my_table.col1, my_table.col2, my_table.col3 FROM my_table"
    );
    assert!(bindings.is_empty());
    Ok(())
}

#[test]
fn test_select_specific_fields() -> Result<(), Box<dyn StdError>> {
    let registry = registry(true);
    let uri = mock_uri("fields[my_table]=col1,col2");
    let (query, bindings) = QueryBuilder::new(schema(&registry))
        .query(&parse(&registry, &uri))?
        .ok_or("query should be satisfiable")?;

    assert_eq!(
        query,
        "SELECT my_table.id, my_table.col1, my_table.col2 FROM my_table"
    );
    assert!(bindings.is_empty());
    Ok(())
}

#[test]
fn test_filter_single_condition() -> Result<(), Box<dyn StdError>> {
    let registry = registry(true);
    let uri = mock_uri("filter[col1]=eq:value1");
    let (query, bindings) = QueryBuilder::new(schema(&registry))
        .query(&parse(&registry, &uri))?
        .ok_or("query should be satisfiable")?;

    assert_eq!(
        query,
        "SELECT my_table.id, my_table.col1, my_table.col2, my_table.col3 FROM my_table WHERE my_table.col1 = ?1"
    );
    assert_eq!(bindings, vec![Attribute::Text("value1".to_string())]);
    Ok(())
}

#[test]
fn test_filter_multiple_conditions() -> Result<(), Box<dyn StdError>> {
    let registry = registry(true);
    let uri = mock_uri("filter[col1]=eq:value1&filter[col2]=neq:value2");
    let (query, bindings) = QueryBuilder::new(schema(&registry))
        .query(&parse(&registry, &uri))?
        .ok_or("query should be satisfiable")?;

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
    Ok(())
}

#[test]
fn test_filter_with_like_operator() -> Result<(), Box<dyn StdError>> {
    let registry = registry(true);
    let uri = mock_uri("filter[col1]=like:keyword");
    let (query, bindings) = QueryBuilder::new(schema(&registry))
        .query(&parse(&registry, &uri))?
        .ok_or("query should be satisfiable")?;

    assert_eq!(
        query,
        "SELECT my_table.id, my_table.col1, my_table.col2, my_table.col3 FROM my_table WHERE my_table.col1 LIKE ?1"
    );
    assert_eq!(bindings, vec![Attribute::Text("%keyword%".to_string())]);
    Ok(())
}

#[test]
fn test_sort_single_field() -> Result<(), Box<dyn StdError>> {
    let registry = registry(true);
    let uri = mock_uri("sort=-col1");
    let (query, bindings) = QueryBuilder::new(schema(&registry))
        .query(&parse(&registry, &uri))?
        .ok_or("query should be satisfiable")?;

    assert_eq!(
        query,
        "SELECT my_table.id, my_table.col1, my_table.col2, my_table.col3 FROM my_table ORDER BY my_table.col1 DESC"
    );
    assert!(bindings.is_empty());
    Ok(())
}

#[test]
fn test_sort_multiple_fields() -> Result<(), Box<dyn StdError>> {
    let registry = registry(true);
    let uri = mock_uri("sort=-col1,col2");
    let (query, bindings) = QueryBuilder::new(schema(&registry))
        .query(&parse(&registry, &uri))?
        .ok_or("query should be satisfiable")?;

    assert_eq!(
        query,
        "SELECT my_table.id, my_table.col1, my_table.col2, my_table.col3 FROM my_table ORDER BY my_table.col1 DESC, my_table.col2 ASC"
    );
    assert!(bindings.is_empty());
    Ok(())
}

#[test]
fn test_pagination() -> Result<(), Box<dyn StdError>> {
    let registry = registry(true);
    let uri = mock_uri("page[number]=2&page[size]=10");
    let (query, bindings) = QueryBuilder::new(schema(&registry))
        .query(&parse(&registry, &uri))?
        .ok_or("query should be satisfiable")?;

    assert_eq!(
        query,
        "SELECT my_table.id, my_table.col1, my_table.col2, my_table.col3 FROM my_table LIMIT 10 OFFSET 10"
    );
    assert!(bindings.is_empty());
    Ok(())
}

#[test]
fn test_complex_query_with_all_features() -> Result<(), Box<dyn StdError>> {
    let registry = registry(true);
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
    let (query, bindings) = QueryBuilder::new(schema(&registry))
        .query(&parse(&registry, &uri))?
        .ok_or("query should be satisfiable")?;

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
    Ok(())
}

#[test]
fn test_find_with_all_fields() -> Result<(), Box<dyn StdError>> {
    let registry = registry(true);
    let uri = mock_uri("");
    let (query, bindings) = QueryBuilder::new(schema(&registry))
        .find(Identifier::Integer(1), &parse(&registry, &uri))?;

    assert_eq!(
        query,
        "SELECT my_table.id, my_table.col1, my_table.col2, my_table.col3 FROM my_table WHERE id = ?1"
    );
    assert_eq!(bindings, vec![Attribute::Integer(1)]);
    Ok(())
}

#[test]
fn test_find_with_specific_fields() -> Result<(), Box<dyn StdError>> {
    let registry = registry(true);
    let uri = mock_uri("fields[my_table]=col1,col2");
    let (query, bindings) = QueryBuilder::new(schema(&registry))
        .find(Identifier::Integer(1), &parse(&registry, &uri))?;

    assert_eq!(
        query,
        "SELECT my_table.id, my_table.col1, my_table.col2 FROM my_table WHERE id = ?1"
    );
    assert_eq!(bindings, vec![Attribute::Integer(1)]);
    Ok(())
}

#[test]
fn test_insert_single_field() -> Result<(), Box<dyn StdError>> {
    let registry = registry(true);
    let uri = mock_uri("");
    let attributes = Attributes::from_iter([("col1", Attribute::Text("value1".to_string()))]);
    let (query, bindings) =
        QueryBuilder::new(schema(&registry)).insert(attributes, &parse(&registry, &uri))?;

    assert_eq!(
        query,
        "INSERT INTO my_table(col1) VALUES (?1) RETURNING id, col1, col2, col3"
    );
    assert_eq!(bindings, vec![Attribute::Text("value1".to_string())]);
    Ok(())
}

#[test]
fn test_insert_multiple_fields() -> Result<(), Box<dyn StdError>> {
    let registry = registry(true);
    let uri = mock_uri("");
    let attributes = Attributes::from_iter([
        ("col1", Attribute::Text("value1".to_string())),
        ("col2", Attribute::Integer(42)),
    ]);
    let (query, bindings) =
        QueryBuilder::new(schema(&registry)).insert(attributes, &parse(&registry, &uri))?;

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
    Ok(())
}

#[test]
fn test_insert_with_returning_fields() -> Result<(), Box<dyn StdError>> {
    let registry = registry(true);
    let uri = mock_uri("fields[my_table]=col1");
    let attributes = Attributes::from_iter([("col1", Attribute::Text("value1".to_string()))]);
    let (query, bindings) =
        QueryBuilder::new(schema(&registry)).insert(attributes, &parse(&registry, &uri))?;

    assert_eq!(
        query,
        "INSERT INTO my_table(col1) VALUES (?1) RETURNING id, col1"
    );
    assert_eq!(bindings, vec![Attribute::Text("value1".to_string())]);
    Ok(())
}

#[test]
fn test_insert_with_empty_attributes() -> Result<(), Box<dyn StdError>> {
    let registry = registry(true);
    let uri = mock_uri("");
    let (query, bindings) =
        QueryBuilder::new(schema(&registry)).insert(Attributes::new(), &parse(&registry, &uri))?;

    assert_eq!(
        query,
        "INSERT INTO my_table() VALUES () RETURNING id, col1, col2, col3"
    );
    assert!(bindings.is_empty());
    Ok(())
}

#[test]
fn test_update_single_field() -> Result<(), Box<dyn StdError>> {
    let registry = registry(true);
    let uri = mock_uri("");
    let attributes = Attributes::from_iter([("col1", Attribute::Text("new_value".to_string()))]);
    let (query, bindings) = QueryBuilder::new(schema(&registry)).update(
        Identifier::Integer(1),
        attributes,
        &parse(&registry, &uri),
    )?;

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
    Ok(())
}

#[test]
fn test_update_multiple_fields() -> Result<(), Box<dyn StdError>> {
    let registry = registry(true);
    let uri = mock_uri("");
    let attributes = Attributes::from_iter([
        ("col1", Attribute::Text("new_value".to_string())),
        ("col2", Attribute::Integer(42)),
    ]);
    let (query, bindings) = QueryBuilder::new(schema(&registry)).update(
        Identifier::Integer(1),
        attributes,
        &parse(&registry, &uri),
    )?;

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
    Ok(())
}

#[test]
fn test_update_with_returning_fields() -> Result<(), Box<dyn StdError>> {
    let registry = registry(true);
    let uri = mock_uri("fields[my_table]=col1");
    let attributes = Attributes::from_iter([("col1", Attribute::Text("new_value".to_string()))]);
    let (query, bindings) = QueryBuilder::new(schema(&registry)).update(
        Identifier::Integer(1),
        attributes,
        &parse(&registry, &uri),
    )?;

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
    Ok(())
}

#[test]
fn test_update_with_empty_attributes() -> Result<(), Box<dyn StdError>> {
    let registry = registry(true);
    let uri = mock_uri("");
    let (query, bindings) = QueryBuilder::new(schema(&registry)).update(
        Identifier::Integer(1),
        Attributes::new(),
        &parse(&registry, &uri),
    )?;

    assert_eq!(
        query,
        "UPDATE my_table WHERE id = ?1 RETURNING id, col1, col2, col3"
    );
    assert_eq!(bindings, vec![Attribute::Integer(1)]);
    Ok(())
}

#[test]
fn test_update_batch_scopes_by_filter() -> Result<(), Box<dyn StdError>> {
    let registry = registry(true);
    let uri = mock_uri("");
    let attributes = Attributes::from_iter([("col1", Attribute::Null)]);
    let (query, bindings) = QueryBuilder::new(schema(&registry))
        .update_batch(attributes, &scoped_to_id(&registry, &uri, 5))?
        .ok_or("update should be satisfiable")?;

    assert_eq!(
        query,
        "UPDATE my_table SET col1 = ?1 WHERE my_table.id = ?2 RETURNING id, col1, col2, col3"
    );
    assert_eq!(bindings, vec![Attribute::Null, Attribute::Integer(5)]);
    Ok(())
}

#[test]
fn test_update_batch_with_in_filter() -> Result<(), Box<dyn StdError>> {
    let registry = registry(true);
    let uri = mock_uri("");
    let mut parameters = parse(&registry, &uri);
    parameters.filter = Some(FilterParameters::from([(
        "col3",
        vec![FilterValue::In(IndexSet::from([
            Attribute::Integer(1),
            Attribute::Integer(2),
        ]))],
    )]));
    let attributes = Attributes::from_iter([("col1", Attribute::Integer(7))]);
    let (query, bindings) = QueryBuilder::new(schema(&registry))
        .update_batch(attributes, &parameters)?
        .ok_or("update should be satisfiable")?;

    assert_eq!(
        query,
        "UPDATE my_table SET col1 = ?1 WHERE my_table.col3 IN (?2,?3) RETURNING id, col1, col2, col3"
    );
    assert_eq!(
        bindings,
        vec![
            Attribute::Integer(7),
            Attribute::Integer(1),
            Attribute::Integer(2)
        ]
    );
    Ok(())
}

#[test]
fn test_delete() {
    let registry = registry(true);
    let (query, bindings) = QueryBuilder::new(schema(&registry)).delete(Identifier::Integer(1));

    assert_eq!(query, "DELETE FROM my_table WHERE id = ?1");
    assert_eq!(bindings, vec![Attribute::Integer(1)]);
}

#[test]
fn test_insert_batch_multiple_rows() -> Result<(), Box<dyn StdError>> {
    let registry = registry(true);
    let uri = mock_uri("");
    let rows = vec![
        Attributes::from_iter([
            ("col1", Attribute::Text("a".to_string())),
            ("col2", Attribute::Text("b".to_string())),
        ]),
        Attributes::from_iter([
            ("col1", Attribute::Text("c".to_string())),
            ("col2", Attribute::Text("d".to_string())),
        ]),
    ];
    let (query, bindings) =
        QueryBuilder::new(schema(&registry)).insert_batch(rows, &parse(&registry, &uri))?;

    assert_eq!(
        query,
        "INSERT INTO my_table(col1, col2) VALUES (?1, ?2), (?3, ?4) RETURNING id, col1, col2, col3"
    );
    assert_eq!(
        bindings,
        vec![
            Attribute::Text("a".to_string()),
            Attribute::Text("b".to_string()),
            Attribute::Text("c".to_string()),
            Attribute::Text("d".to_string()),
        ]
    );
    Ok(())
}

#[test]
fn test_insert_batch_rejects_heterogeneous_columns() {
    let registry = registry(true);
    let uri = mock_uri("");
    let rows = vec![
        Attributes::from_iter([("col1", Attribute::Text("a".to_string()))]),
        Attributes::from_iter([("col2", Attribute::Text("b".to_string()))]),
    ];
    let result = QueryBuilder::new(schema(&registry)).insert_batch(rows, &parse(&registry, &uri));

    assert!(matches!(result, Err(Error::InvalidOperation { .. })));
}

#[test]
fn test_delete_batch_scoped_by_filter() -> Result<(), Box<dyn StdError>> {
    let registry = registry(true);
    let uri = mock_uri("");
    let (query, bindings) = QueryBuilder::new(schema(&registry))
        .delete_batch(&scoped_to_id(&registry, &uri, 5))?
        .ok_or("delete should be satisfiable")?;

    assert_eq!(query, "DELETE FROM my_table WHERE my_table.id = ?1");
    assert_eq!(bindings, vec![Attribute::Integer(5)]);
    Ok(())
}

#[test]
fn test_delete_batch_unscoped() -> Result<(), Box<dyn StdError>> {
    let registry = registry(true);
    let uri = mock_uri("");
    let (query, bindings) = QueryBuilder::new(schema(&registry))
        .delete_batch(&parse(&registry, &uri))?
        .ok_or("delete should be satisfiable")?;

    assert_eq!(query, "DELETE FROM my_table");
    assert!(bindings.is_empty());
    Ok(())
}

#[test]
fn test_filter_with_like_operator_on_non_text_attribute() {
    let registry = registry(true);
    let uri = mock_uri("filter[col3]=like:1");
    let result = QueryBuilder::new(schema(&registry)).query(&parse(&registry, &uri));

    assert!(result.is_err());
}

#[test]
fn test_search_with_single_term() -> Result<(), Box<dyn StdError>> {
    let registry = registry(true);
    let uri = mock_uri("search=a-value-to-search");
    let (query, bindings) = QueryBuilder::new(schema(&registry))
        .query(&parse(&registry, &uri))?
        .ok_or("query should be satisfiable")?;

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
    Ok(())
}

#[test]
fn test_search_with_multiple_terms() -> Result<(), Box<dyn StdError>> {
    let registry = registry(true);
    let uri = mock_uri("search=a-value,another-value");
    let (query, bindings) = QueryBuilder::new(schema(&registry))
        .query(&parse(&registry, &uri))?
        .ok_or("query should be satisfiable")?;

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
    Ok(())
}

#[test]
fn test_search_on_table_without_text_index() {
    let registry = registry(false);
    let uri = mock_uri("search=a-value-to-search");
    let parameters =
        QueryParameters::parse(&uri, schema(&registry), &registry).expect("query parses");
    let result = QueryBuilder::new(schema(&registry)).query(&parameters);

    assert!(result.is_err());
}

// --- Unsatisfiable filters ---

#[test]
fn test_query_with_empty_in_is_unsatisfiable() -> Result<(), Box<dyn StdError>> {
    let registry = registry(true);
    let uri = mock_uri("");
    let mut parameters = parse(&registry, &uri);
    parameters.filter = Some(FilterParameters::from([(
        "col3",
        vec![FilterValue::In(IndexSet::new())],
    )]));
    let built = QueryBuilder::new(schema(&registry)).query(&parameters)?;

    assert_eq!(built, None);
    Ok(())
}

#[test]
fn test_query_with_empty_not_in_is_omitted() -> Result<(), Box<dyn StdError>> {
    let registry = registry(true);
    let uri = mock_uri("");
    let mut parameters = parse(&registry, &uri);
    parameters.filter = Some(FilterParameters::from([(
        "col3",
        vec![FilterValue::NotIn(IndexSet::new())],
    )]));
    let (query, bindings) = QueryBuilder::new(schema(&registry))
        .query(&parameters)?
        .ok_or("query should be satisfiable")?;

    assert_eq!(
        query,
        "SELECT my_table.id, my_table.col1, my_table.col2, my_table.col3 FROM my_table"
    );
    assert!(bindings.is_empty());
    Ok(())
}

#[test]
fn test_update_batch_with_empty_in_is_unsatisfiable() -> Result<(), Box<dyn StdError>> {
    let registry = registry(true);
    let uri = mock_uri("");
    let mut parameters = parse(&registry, &uri);
    parameters.filter = Some(FilterParameters::from([(
        "col3",
        vec![FilterValue::In(IndexSet::new())],
    )]));
    let attributes = Attributes::from_iter([("col1", Attribute::Integer(7))]);
    let built = QueryBuilder::new(schema(&registry)).update_batch(attributes, &parameters)?;

    assert_eq!(built, None);
    Ok(())
}

#[test]
fn test_delete_batch_with_empty_in_is_unsatisfiable() -> Result<(), Box<dyn StdError>> {
    let registry = registry(true);
    let uri = mock_uri("");
    let mut parameters = parse(&registry, &uri);
    parameters.filter = Some(FilterParameters::from([(
        "col3",
        vec![FilterValue::In(IndexSet::new())],
    )]));
    let built = QueryBuilder::new(schema(&registry)).delete_batch(&parameters)?;

    assert_eq!(built, None);
    Ok(())
}
