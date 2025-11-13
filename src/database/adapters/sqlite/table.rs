use crate::database::{
    adapters::sqlite::{Connection, QueryBuilder},
    error::Error,
    schema::TableSchema,
    table::Table as TableInterface,
};

use std::sync::{Arc, Mutex, MutexGuard};

pub struct Table<'sch> {
    pub table_schema: &'sch TableSchema<'sch>,
    pub connection: Arc<Mutex<Connection>>,
}

impl<'sch> TableInterface<'sch, Connection, QueryBuilder<'sch>> for Table<'sch> {
    fn new(table_schema: &'sch TableSchema, connection: Arc<Mutex<Connection>>) -> Self {
        Self {
            table_schema,
            connection,
        }
    }

    fn schema(&self) -> &'sch TableSchema<'sch> {
        self.table_schema
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, Error> {
        self.connection
            .lock()
            .map_err(|err| Error::DatabaseFailure {
                message: err.to_string(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::schema::{IdentifierType, PrimaryKey};
    use crate::database::{attributes::{Attribute, Attributes}, schema::{AttributeType, TableSchema}, query_parameters::QueryParameters};
    use crate::http_wrappers::Uri;
    use rusqlite::Connection;
    use std::error::Error as StdError;

    fn mock_params(query: &str) -> Result<QueryParameters, Box<dyn StdError>> {
        let uri = format!("http://host.com/resource?{}", query).parse::<Uri>()?;
        Ok(QueryParameters::parse(&uri)?)
    }

    struct Context<'sch> {
        connection: Arc<Mutex<Connection>>,
        schema: TableSchema<'sch>,
    }

    impl<'sch> Context<'sch> {
        pub(super) fn new() -> Self {
            let connection = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
            let schema = TableSchema {
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

            connection
                .lock()
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
            Context { connection, schema }
        }

        pub(super) fn table(&self) -> Table<'_> {
            Table {
                table_schema: &self.schema,
                connection: self.connection.clone(),
            }
        }

        pub(super) fn seed_table(&self) -> Result<Table<'_>, Error> {
            let table = self.table();

            table.insert(
                [
                    (
                        "col1".to_string(),
                        Attribute::Text("The quick brown fox".to_string()),
                    ),
                    (
                        "col2".to_string(),
                        Attribute::Text("jumps over the lazy dog".to_string()),
                    ),
                    ("col3".to_string(), Attribute::Integer(42)),
                ]
                .into(),
                &QueryParameters::default(),
            )?;
            table.insert(
                [
                    (
                        "col1".to_string(),
                        Attribute::Text("The five boxing wizards".to_string()),
                    ),
                    (
                        "col2".to_string(),
                        Attribute::Text("jump quickly".to_string()),
                    ),
                    ("col3".to_string(), Attribute::Integer(1000)),
                ]
                .into(),
                &QueryParameters::default(),
            )?;
            table.insert(
                [
                    (
                        "col1".to_string(),
                        Attribute::Text("Pack my box".to_string()),
                    ),
                    (
                        "col2".to_string(),
                        Attribute::Text("with five dozen liquor jugs".to_string()),
                    ),
                    ("col3".to_string(), Attribute::Integer(-1000)),
                ]
                .into(),
                &QueryParameters::default(),
            )?;

            Ok(table)
        }
    }

    #[test]
    fn test_query_without_records() {
        let context = Context::new();
        let parameters = QueryParameters::default();
        let result = context.table().query(&parameters);

        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_first_without_records() {
        let context = Context::new();
        let parameters = QueryParameters::default();
        let result = context.table().first(&parameters);

        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_find_without_records() {
        let context = Context::new();
        let parameters = QueryParameters::default();
        let result = context.table().find(1, &parameters);

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), Error::RecordNotFound));
    }

    #[test]
    fn test_query() -> Result<(), Box<dyn StdError>> {
        let context = Context::new();
        let table = context.seed_table()?;

        let default_params_result = table.query(&QueryParameters::default())?;

        assert_eq!(default_params_result.len(), 3);

        let find_single_params =
            mock_params("filter[col1]=eq:The%20quick%20brown%20fox&filter[col3]=eq:42")?;
        let find_single_result = table.query(&find_single_params)?;

        assert_eq!(find_single_result.len(), 1);
        assert_eq!(
            find_single_result[0].attributes.get("col1").unwrap(),
            &Attribute::Text("The quick brown fox".to_string())
        );

        let find_many_params = mock_params("filter[col2]=like:jump")?;
        let find_many_result = table.query(&find_many_params)?;

        assert_eq!(find_many_result.len(), 2);
        assert_eq!(
            find_many_result[0].attributes.get("col3").unwrap(),
            &Attribute::Integer(42)
        );
        assert_eq!(
            find_many_result[1].attributes.get("col3").unwrap(),
            &Attribute::Integer(1000)
        );

        let find_none_params =
            mock_params("filter[col3]=lt:50&filter[col1]=like:I%20am%20not%20here")?;
        let find_none_result = table.query(&find_none_params)?;

        assert_eq!(find_none_result.len(), 0);

        let text_search_params = mock_params("search=five,box")?;
        let text_search_result = table.query(&text_search_params)?;

        assert_eq!(text_search_result.len(), 2);
        assert_eq!(
            text_search_result[0].attributes.get("col3").unwrap(),
            &Attribute::Integer(1000)
        );
        assert_eq!(
            text_search_result[1].attributes.get("col3").unwrap(),
            &Attribute::Integer(-1000)
        );

        Ok(())
    }

    #[test]
    fn test_first() -> Result<(), Box<dyn StdError>> {
        let context = Context::new();
        let table = context.seed_table()?;
        let result = table.first(&QueryParameters::default())?;

        assert!(result.is_some());
        assert_eq!(
            result.unwrap().attributes.get("col1"),
            Some(&Attribute::Text("The quick brown fox".to_string()))
        );

        Ok(())
    }

    #[test]
    fn test_find() {
        let context = Context::new();
        let parameters = QueryParameters::default();
        let result = context.table().find(1, &parameters);

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), Error::RecordNotFound));
    }

    #[test]
    fn test_insert() {
        let context = Context::new();
        let attributes =
            Attributes::from([("col1".to_string(), Attribute::Text("value1".to_string()))]);
        let parameters = QueryParameters::default();
        let result = context.table().insert(attributes, &parameters);

        assert!(result.is_ok());
        let row = result.unwrap();
        assert_eq!(
            row.attributes.get("col1").unwrap(),
            &Attribute::Text("value1".to_string())
        );
    }

    #[test]
    fn test_update() {
        let context = Context::new();
        let insert_attributes =
            Attributes::from([("col1".to_string(), Attribute::Text("value1".to_string()))]);
        let insert_parameters = QueryParameters::default();
        context
            .table()
            .insert(insert_attributes, &insert_parameters)
            .unwrap();

        let update_attributes =
            Attributes::from([("col1".to_string(), Attribute::Text("new_value".to_string()))]);
        let update_parameters = QueryParameters::default();
        let result = context
            .table()
            .update(1, update_attributes, &update_parameters);

        assert!(result.is_ok());
        let row = result.unwrap();
        assert_eq!(
            row.attributes.get("col1").unwrap(),
            &Attribute::Text("new_value".to_string())
        );
    }

    #[test]
    fn test_delete() {
        let context = Context::new();
        let attributes =
            Attributes::from([("col1".to_string(), Attribute::Text("value1".to_string()))]);
        let parameters = QueryParameters::default();
        context.table().insert(attributes, &parameters).unwrap();

        let result = context.table().delete(1);
        assert!(result.is_ok());

        let find_result = context.table().find(1, &parameters);
        assert!(find_result.is_err());
        assert!(matches!(find_result.unwrap_err(), Error::RecordNotFound));
    }
}
