use crate::database::{
    adapters::sqlite::{Connection, QueryBuilder},
    schema::TableSchema,
    table::Table as TableInterface,
};

pub struct Table<'sch, 'req> {
    pub table_schema: &'sch TableSchema<'sch>,
    pub connection: &'req Connection,
}

impl<'sch, 'req> TableInterface<'sch, 'req, Connection, QueryBuilder<'sch>> for Table<'sch, 'req> {
    fn new(table_schema: &'sch TableSchema<'sch>, connection: &'req Connection) -> Self {
        Self {
            table_schema,
            connection,
        }
    }

    fn schema(&self) -> &'sch TableSchema<'sch> {
        self.table_schema
    }

    fn connection(&self) -> &'req Connection {
        self.connection
    }
}

#[cfg(test)]
mod tests {
    use crate::database::adapters::sqlite::Pool;
    use crate::database::connection::Connection as ConnectionInterface;
    use crate::database::{
        adapters::SqliteAdapter,
        attributes::{Attribute, Attributes, Identifier},
        error::Error,
        query_parameters::{FilterParameters, FilterValue, QueryParameters},
        registry::Registry as DatabaseRegistry,
        schema::{AttributeType, IdentifierType, PrimaryKey, TableSchema},
        table::Table,
    };
    use crate::http_wrappers::Uri;
    use std::error::Error as StdError;

    type Registry = DatabaseRegistry<'static, SqliteAdapter>;

    static MY_SCHEMA: TableSchema = TableSchema {
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
        text_index: true,
    };

    static SCHEMAS: [&TableSchema; 1] = [&MY_SCHEMA];

    fn registry() -> Registry {
        let registry = Registry::try_new(Pool::memory().unwrap(), &SCHEMAS).unwrap();

        registry
            .acquire()
            .unwrap()
            .execute_batch(
                "\
                CREATE TABLE my_table (id INTEGER PRIMARY KEY, col1 TEXT, col2 TEXT, col3 NUMBER); \
                CREATE VIRTUAL TABLE my_table_fts USING fts5(col1, col2, tokenize='trigram'); \
                CREATE TRIGGER my_table_fts_insert AFTER INSERT ON my_table BEGIN \
                  INSERT INTO my_table_fts(rowid, col1, col2) VALUES (new.id, new.col1, new.col2); \
                END; \
                CREATE TRIGGER my_table_fts_update AFTER UPDATE ON my_table BEGIN \
                  UPDATE my_table_fts SET col1 = new.col1, col2 = new.col2 WHERE rowid = new.id; \
                END; \
                CREATE TRIGGER my_table_fts_delete AFTER DELETE ON my_table BEGIN \
                  DELETE FROM my_table_fts WHERE rowid = old.id; \
                END; \
                ",
            )
            .unwrap();

        registry
    }

    fn seeded_registry() -> Result<Registry, Box<dyn StdError>> {
        let registry = registry();

        {
            let connection = registry.acquire()?;
            let table = registry.table("my_table", &connection)?;

            for (col1, col2, col3) in [
                ("The quick brown fox", "jumps over the lazy dog", 42),
                ("The five boxing wizards", "jump quickly", 1000),
                ("Pack my box", "with five dozen liquor jugs", -1000),
            ] {
                table.insert(
                    Attributes::from_iter([
                        ("col1".to_string(), Attribute::Text(col1.to_string())),
                        ("col2".to_string(), Attribute::Text(col2.to_string())),
                        ("col3".to_string(), Attribute::Integer(col3)),
                    ]),
                    &QueryParameters::new(&MY_SCHEMA),
                )?;
            }
        }

        Ok(registry)
    }

    fn mock_uri(query: &str) -> Uri {
        format!("http://host.com/my_table?{}", query)
            .parse::<Uri>()
            .unwrap()
    }

    #[test]
    fn test_query_without_records() {
        let registry = registry();
        let connection = registry.acquire().unwrap();
        let result = registry
            .table("my_table", &connection)
            .unwrap()
            .query(&QueryParameters::new(&MY_SCHEMA));

        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_first_without_records() {
        let registry = registry();
        let connection = registry.acquire().unwrap();
        let result = registry
            .table("my_table", &connection)
            .unwrap()
            .first(&QueryParameters::new(&MY_SCHEMA));

        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_find_without_records() {
        let registry = registry();
        let connection = registry.acquire().unwrap();
        let result = registry
            .table("my_table", &connection)
            .unwrap()
            .find(Identifier::Integer(1), &QueryParameters::new(&MY_SCHEMA));

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), Error::RecordNotFound));
    }

    #[test]
    fn test_query() -> Result<(), Box<dyn StdError>> {
        let registry = seeded_registry()?;
        let connection = registry.acquire()?;
        let table = registry.table("my_table", &connection)?;

        let default_result = table.query(&QueryParameters::new(&MY_SCHEMA))?;
        assert_eq!(default_result.len(), 3);

        let single_uri = mock_uri("filter[col1]=eq:The%20quick%20brown%20fox&filter[col3]=eq:42");
        let single = table.query(&QueryParameters::parse(&single_uri, &MY_SCHEMA, &registry)?)?;
        assert_eq!(single.len(), 1);
        assert_eq!(
            single[0].attributes.get("col1").unwrap(),
            &Attribute::Text("The quick brown fox".to_string())
        );

        let many_uri = mock_uri("filter[col2]=like:jump");
        let many = table.query(&QueryParameters::parse(&many_uri, &MY_SCHEMA, &registry)?)?;
        assert_eq!(many.len(), 2);
        assert_eq!(
            many[0].attributes.get("col3").unwrap(),
            &Attribute::Integer(42)
        );
        assert_eq!(
            many[1].attributes.get("col3").unwrap(),
            &Attribute::Integer(1000)
        );

        let none_uri = mock_uri("filter[col3]=lt:50&filter[col1]=like:I%20am%20not%20here");
        let none = table.query(&QueryParameters::parse(&none_uri, &MY_SCHEMA, &registry)?)?;
        assert_eq!(none.len(), 0);

        let search_uri = mock_uri("search=five,box");
        let search = table.query(&QueryParameters::parse(&search_uri, &MY_SCHEMA, &registry)?)?;
        assert_eq!(search.len(), 2);
        assert_eq!(
            search[0].attributes.get("col3").unwrap(),
            &Attribute::Integer(1000)
        );
        assert_eq!(
            search[1].attributes.get("col3").unwrap(),
            &Attribute::Integer(-1000)
        );

        Ok(())
    }

    #[test]
    fn test_first() -> Result<(), Box<dyn StdError>> {
        let registry = seeded_registry()?;
        let connection = registry.acquire()?;
        let result = registry
            .table("my_table", &connection)?
            .first(&QueryParameters::new(&MY_SCHEMA))?;

        assert!(result.is_some());
        assert_eq!(
            result.unwrap().attributes.get("col1"),
            Some(&Attribute::Text("The quick brown fox".to_string()))
        );

        Ok(())
    }

    #[test]
    fn test_find() {
        let registry = registry();
        let connection = registry.acquire().unwrap();
        let result = registry
            .table("my_table", &connection)
            .unwrap()
            .find(Identifier::Integer(1), &QueryParameters::new(&MY_SCHEMA));

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), Error::RecordNotFound));
    }

    #[test]
    fn test_insert() -> Result<(), Box<dyn StdError>> {
        let registry = registry();
        let connection = registry.acquire()?;
        let result = registry.table("my_table", &connection)?.insert(
            Attributes::from_iter([("col1".to_string(), Attribute::Text("value1".to_string()))]),
            &QueryParameters::new(&MY_SCHEMA),
        )?;

        assert_eq!(
            result.attributes.get("col1").unwrap(),
            &Attribute::Text("value1".to_string())
        );

        Ok(())
    }

    #[test]
    fn test_update() -> Result<(), Box<dyn StdError>> {
        let registry = registry();
        let connection = registry.acquire()?;
        let table = registry.table("my_table", &connection)?;

        table.insert(
            Attributes::from_iter([("col1".to_string(), Attribute::Text("value1".to_string()))]),
            &QueryParameters::new(&MY_SCHEMA),
        )?;

        let result = table.update(
            Identifier::Integer(1),
            Attributes::from_iter([("col1".to_string(), Attribute::Text("new_value".to_string()))]),
            &QueryParameters::new(&MY_SCHEMA),
        )?;

        assert_eq!(
            result.attributes.get("col1").unwrap(),
            &Attribute::Text("new_value".to_string())
        );

        Ok(())
    }

    #[test]
    fn test_delete() -> Result<(), Box<dyn StdError>> {
        let registry = registry();
        let connection = registry.acquire()?;
        let table = registry.table("my_table", &connection)?;

        table.insert(
            Attributes::from_iter([("col1".to_string(), Attribute::Text("value1".to_string()))]),
            &QueryParameters::new(&MY_SCHEMA),
        )?;

        table.delete(Identifier::Integer(1))?;

        let result = table.find(Identifier::Integer(1), &QueryParameters::new(&MY_SCHEMA));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), Error::RecordNotFound));

        Ok(())
    }

    #[test]
    fn test_insert_batch() -> Result<(), Box<dyn StdError>> {
        let registry = registry();
        let connection = registry.acquire()?;
        let table = registry.table("my_table", &connection)?;

        let rows = vec![
            Attributes::from_iter([("col1".to_string(), Attribute::Text("a".to_string()))]),
            Attributes::from_iter([("col1".to_string(), Attribute::Text("b".to_string()))]),
        ];
        let inserted = table.insert_batch(rows, &QueryParameters::new(&MY_SCHEMA))?;

        assert_eq!(inserted.len(), 2);
        assert_eq!(table.query(&QueryParameters::new(&MY_SCHEMA))?.len(), 2);

        Ok(())
    }

    #[test]
    fn test_insert_batch_empty_is_a_noop() -> Result<(), Box<dyn StdError>> {
        let registry = registry();
        let connection = registry.acquire()?;
        let table = registry.table("my_table", &connection)?;

        let inserted = table.insert_batch(Vec::new(), &QueryParameters::new(&MY_SCHEMA))?;

        assert!(inserted.is_empty());
        assert!(table.query(&QueryParameters::new(&MY_SCHEMA))?.is_empty());

        Ok(())
    }

    #[test]
    fn test_delete_batch_scoped_by_filter() -> Result<(), Box<dyn StdError>> {
        let registry = seeded_registry()?;
        let connection = registry.acquire()?;
        let table = registry.table("my_table", &connection)?;

        let mut parameters = QueryParameters::new(&MY_SCHEMA);
        parameters.filter = Some(FilterParameters::from([(
            "col3",
            vec![FilterValue::LessThan(Attribute::Integer(0))],
        )]));
        table.delete_batch(&parameters)?;

        assert_eq!(table.query(&QueryParameters::new(&MY_SCHEMA))?.len(), 2);

        Ok(())
    }

    #[test]
    fn test_delete_batch_unscoped_clears_table() -> Result<(), Box<dyn StdError>> {
        let registry = seeded_registry()?;
        let connection = registry.acquire()?;
        let table = registry.table("my_table", &connection)?;

        table.delete_batch(&QueryParameters::new(&MY_SCHEMA))?;

        assert!(table.query(&QueryParameters::new(&MY_SCHEMA))?.is_empty());

        Ok(())
    }

    #[test]
    fn test_transaction_rolls_back_on_error() -> Result<(), Box<dyn StdError>> {
        let registry = registry();
        let connection = registry.acquire()?;
        let table = registry.table("my_table", &connection)?;
        let parameters = QueryParameters::new(&MY_SCHEMA);

        let result: Result<(), Error> = connection.transaction(|| {
            table.insert(
                Attributes::from_iter([(
                    "col1".to_string(),
                    Attribute::Text("rolled-back".to_string()),
                )]),
                &parameters,
            )?;

            Err(Error::DataLoadingError {
                message: "deliberate failure".to_string(),
            })
        });

        assert!(result.is_err());
        assert!(table.query(&parameters)?.is_empty());

        Ok(())
    }
}
