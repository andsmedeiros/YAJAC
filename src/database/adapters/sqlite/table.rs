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
        attributes::{Attribute, Identifier, Row},
        connection_manager::ConnectionManager,
        error::Error,
        query_parameters::{FilterParameters, FilterValue, QueryParameters},
        registry::Registry,
        schema::{AttributeType, SchemaBuilder, TableSchema},
        table::Table,
    };
    use crate::http_wrappers::Uri;
    use std::error::Error as StdError;

    type Manager = ConnectionManager<'static, SqliteAdapter>;

    fn my_schema() -> SchemaBuilder<'static> {
        SchemaBuilder::table("my_table")
            .attribute("col1", AttributeType::Text)
            .attribute("col2", AttributeType::Text)
            .attribute("col3", AttributeType::Integer)
            .text_index()
    }

    fn schema(manager: &Manager) -> &TableSchema<'_> {
        manager
            .registry()
            .schema("my_table")
            .expect("my_table is registered")
    }

    fn manager() -> Manager {
        let manager: Manager = ConnectionManager::new(
            Registry::try_new([my_schema()]).expect("schema set is consistent"),
            Pool::memory().expect("in-memory pool is available"),
        );

        manager
            .acquire()
            .expect("connection is available")
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
            .expect("schema creation succeeds");

        manager
    }

    fn seeded_manager() -> Result<Manager, Box<dyn StdError>> {
        let manager = manager();

        {
            let connection = manager.acquire()?;
            let table = manager.table("my_table", &connection)?;

            for (col1, col2, col3) in [
                ("The quick brown fox", "jumps over the lazy dog", 42),
                ("The five boxing wizards", "jump quickly", 1000),
                ("Pack my box", "with five dozen liquor jugs", -1000),
            ] {
                table.insert(
                    Row::from_iter([
                        ("col1".to_string(), Attribute::Text(col1.to_string())),
                        ("col2".to_string(), Attribute::Text(col2.to_string())),
                        ("col3".to_string(), Attribute::Integer(col3)),
                    ]),
                    &QueryParameters::new(schema(&manager)),
                )?;
            }
        }

        Ok(manager)
    }

    fn mock_uri(query: &str) -> Uri {
        format!("http://host.com/my_table?{}", query)
            .parse::<Uri>()
            .unwrap()
    }

    #[test]
    fn test_query_without_records() {
        let manager = manager();
        let connection = manager.acquire().unwrap();
        let result = manager
            .table("my_table", &connection)
            .unwrap()
            .query(&QueryParameters::new(schema(&manager)));

        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_first_without_records() {
        let manager = manager();
        let connection = manager.acquire().unwrap();
        let result = manager
            .table("my_table", &connection)
            .unwrap()
            .first(&QueryParameters::new(schema(&manager)));

        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_find_without_records() {
        let manager = manager();
        let connection = manager.acquire().unwrap();
        let result = manager.table("my_table", &connection).unwrap().find(
            Identifier::Integer(1),
            &QueryParameters::new(schema(&manager)),
        );

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), Error::RecordNotFound));
    }

    #[test]
    fn test_query() -> Result<(), Box<dyn StdError>> {
        let manager = seeded_manager()?;
        let connection = manager.acquire()?;
        let table = manager.table("my_table", &connection)?;

        let default_result = table.query(&QueryParameters::new(schema(&manager)))?;
        assert_eq!(default_result.len(), 3);

        let single_uri = mock_uri("filter[col1]=eq:The%20quick%20brown%20fox&filter[col3]=eq:42");
        let single = table.query(&QueryParameters::parse(
            &single_uri,
            schema(&manager),
            manager.registry(),
        )?)?;
        assert_eq!(single.len(), 1);
        assert_eq!(
            single[0].get("col1").unwrap(),
            &Attribute::Text("The quick brown fox".to_string())
        );

        let many_uri = mock_uri("filter[col2]=like:jump");
        let many = table.query(&QueryParameters::parse(
            &many_uri,
            schema(&manager),
            manager.registry(),
        )?)?;
        assert_eq!(many.len(), 2);
        assert_eq!(many[0].get("col3").unwrap(), &Attribute::Integer(42));
        assert_eq!(many[1].get("col3").unwrap(), &Attribute::Integer(1000));

        let none_uri = mock_uri("filter[col3]=lt:50&filter[col1]=like:I%20am%20not%20here");
        let none = table.query(&QueryParameters::parse(
            &none_uri,
            schema(&manager),
            manager.registry(),
        )?)?;
        assert_eq!(none.len(), 0);

        let search_uri = mock_uri("search=five,box");
        let search = table.query(&QueryParameters::parse(
            &search_uri,
            schema(&manager),
            manager.registry(),
        )?)?;
        assert_eq!(search.len(), 2);
        assert_eq!(search[0].get("col3").unwrap(), &Attribute::Integer(1000));
        assert_eq!(search[1].get("col3").unwrap(), &Attribute::Integer(-1000));

        Ok(())
    }

    #[test]
    fn test_first() -> Result<(), Box<dyn StdError>> {
        let manager = seeded_manager()?;
        let connection = manager.acquire()?;
        let result = manager
            .table("my_table", &connection)?
            .first(&QueryParameters::new(schema(&manager)))?;

        assert!(result.is_some());
        assert_eq!(
            result.unwrap().get("col1"),
            Some(&Attribute::Text("The quick brown fox".to_string()))
        );

        Ok(())
    }

    #[test]
    fn test_find() {
        let manager = manager();
        let connection = manager.acquire().unwrap();
        let result = manager.table("my_table", &connection).unwrap().find(
            Identifier::Integer(1),
            &QueryParameters::new(schema(&manager)),
        );

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), Error::RecordNotFound));
    }

    #[test]
    fn test_insert() -> Result<(), Box<dyn StdError>> {
        let manager = manager();
        let connection = manager.acquire()?;
        let result = manager.table("my_table", &connection)?.insert(
            Row::from_iter([("col1".to_string(), Attribute::Text("value1".to_string()))]),
            &QueryParameters::new(schema(&manager)),
        )?;

        assert_eq!(
            result.get("col1").unwrap(),
            &Attribute::Text("value1".to_string())
        );

        Ok(())
    }

    #[test]
    fn test_update() -> Result<(), Box<dyn StdError>> {
        let manager = manager();
        let connection = manager.acquire()?;
        let table = manager.table("my_table", &connection)?;

        table.insert(
            Row::from_iter([("col1".to_string(), Attribute::Text("value1".to_string()))]),
            &QueryParameters::new(schema(&manager)),
        )?;

        let result = table.update(
            Identifier::Integer(1),
            Row::from_iter([("col1".to_string(), Attribute::Text("new_value".to_string()))]),
            &QueryParameters::new(schema(&manager)),
        )?;

        assert_eq!(
            result.get("col1").unwrap(),
            &Attribute::Text("new_value".to_string())
        );

        Ok(())
    }

    #[test]
    fn test_delete() -> Result<(), Box<dyn StdError>> {
        let manager = manager();
        let connection = manager.acquire()?;
        let table = manager.table("my_table", &connection)?;

        table.insert(
            Row::from_iter([("col1".to_string(), Attribute::Text("value1".to_string()))]),
            &QueryParameters::new(schema(&manager)),
        )?;

        table.delete(Identifier::Integer(1))?;

        let result = table.find(
            Identifier::Integer(1),
            &QueryParameters::new(schema(&manager)),
        );
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), Error::RecordNotFound));

        Ok(())
    }

    #[test]
    fn test_insert_batch() -> Result<(), Box<dyn StdError>> {
        let manager = manager();
        let connection = manager.acquire()?;
        let table = manager.table("my_table", &connection)?;

        let rows = vec![
            Row::from_iter([("col1".to_string(), Attribute::Text("a".to_string()))]),
            Row::from_iter([("col1".to_string(), Attribute::Text("b".to_string()))]),
        ];
        let inserted = table.insert_batch(rows, &QueryParameters::new(schema(&manager)))?;

        assert_eq!(inserted.len(), 2);
        assert_eq!(
            table.query(&QueryParameters::new(schema(&manager)))?.len(),
            2
        );

        Ok(())
    }

    #[test]
    fn test_insert_batch_empty_is_a_noop() -> Result<(), Box<dyn StdError>> {
        let manager = manager();
        let connection = manager.acquire()?;
        let table = manager.table("my_table", &connection)?;

        let inserted = table.insert_batch(Vec::new(), &QueryParameters::new(schema(&manager)))?;

        assert!(inserted.is_empty());
        assert!(
            table
                .query(&QueryParameters::new(schema(&manager)))?
                .is_empty()
        );

        Ok(())
    }

    #[test]
    fn test_delete_batch_scoped_by_filter() -> Result<(), Box<dyn StdError>> {
        let manager = seeded_manager()?;
        let connection = manager.acquire()?;
        let table = manager.table("my_table", &connection)?;

        let mut parameters = QueryParameters::new(schema(&manager));
        parameters.filter = Some(FilterParameters::from([(
            "col3",
            vec![FilterValue::LessThan(Attribute::Integer(0))],
        )]));
        table.delete_batch(&parameters)?;

        assert_eq!(
            table.query(&QueryParameters::new(schema(&manager)))?.len(),
            2
        );

        Ok(())
    }

    #[test]
    fn test_delete_batch_unscoped_clears_table() -> Result<(), Box<dyn StdError>> {
        let manager = seeded_manager()?;
        let connection = manager.acquire()?;
        let table = manager.table("my_table", &connection)?;

        table.delete_batch(&QueryParameters::new(schema(&manager)))?;

        assert!(
            table
                .query(&QueryParameters::new(schema(&manager)))?
                .is_empty()
        );

        Ok(())
    }

    #[test]
    fn test_transaction_rolls_back_on_error() -> Result<(), Box<dyn StdError>> {
        let manager = manager();
        let connection = manager.acquire()?;
        let table = manager.table("my_table", &connection)?;
        let parameters = QueryParameters::new(schema(&manager));

        let result: Result<(), Error> = connection.transaction(|| {
            table.insert(
                Row::from_iter([(
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
