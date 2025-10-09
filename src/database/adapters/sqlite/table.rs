use crate::database::{
    adapters::sqlite::QueryBuilder,
    attributes::{self, Attribute, Attributes},
    error::Error,
    schema::{AttributeType, TableSchema},
    table::Table as TableInterface,
};
use log::debug;
use base64::{engine::general_purpose::STANDARD as b64, Engine as _};
use rusqlite::{
    types::{ToSql, ToSqlOutput, Value as DatabaseValue, ValueRef},
    Row,
};
use crate::routing::parameters::QueryParameters as Parameters;

pub struct Table<'a> {
    pub table_schema: &'a TableSchema,
    pub connection: &'a Connection,
}

impl<'a> Table<'a> {
    pub fn new(table_schema: &'a TableSchema, connection: &'a Connection) -> Self {
        Self { table_schema, connection }
    }

    fn materialise_record(&self, row: &Row) -> Result<Attributes, Error> {
        let entries = row
            .as_ref()
            .columns()
            .iter()
            .enumerate()
            .map(|(index, column)| -> Result<_, Error> {
                let name = column.name();
                let value = row.get_ref_unwrap(index);

                let attribute_type = self.table_schema.column(name)
                    .expect("Column not found in schema");

                let value = match value {
                    ValueRef::Null =>
                        Attribute::Null,
                    ValueRef::Integer(value) => match attribute_type {
                        AttributeType::Integer =>
                            Attribute::Integer(value),
                        AttributeType::DateTime =>
                            Attribute::DateTime(crate::database::attributes::date_time_from_millis(value, name)?),
                        kind =>
                            crate::database::attributes::inconsistent_schema_error(self.table_schema, name, "Integer", kind)?
                    },
                    ValueRef::Real(value) => match attribute_type {
                        AttributeType::Float =>
                            Attribute::Float(value),
                        kind =>
                            crate::database::attributes::inconsistent_schema_error(self.table_schema, name, "Float", kind)?
                    },
                    ValueRef::Text(value) => {
                        let text = String::from_utf8_lossy(value);
                        match attribute_type {
                            AttributeType::Text =>
                                Attribute::Text(text.to_string()),
                            AttributeType::DateTime =>
                                Attribute::DateTime(crate::database::attributes::date_time_from_iso8601(text.as_ref(), name)?),
                            kind =>
                                crate::database::attributes::inconsistent_schema_error(self.table_schema, name, "Text", kind)?
                        }
                    },
                    ValueRef::Blob(value) => match attribute_type {
                        AttributeType::Text =>
                            Attribute::Text(b64.encode(value)),
                        kind =>
                            crate::database::attributes::inconsistent_schema_error(self.table_schema, name, "Blob", kind)?
                    }
                };
                Ok((name.to_string(), value))
            })
            .collect::<Result<Vec<_>, Error>>()?;

        Ok(Attributes::from_iter(entries.into_iter()))
    }

}

impl<'a> TableInterface for Table<'a> {
    fn schema(&self) -> &TableSchema {
        self.table_schema
    }
    
    fn query(&self, parameters: &Parameters) -> Result<Vec<Attributes>, Error> {
        let (query, bindings) = QueryBuilder::new(self.table_schema).query(parameters)?;
        self.execute(query, bindings)
    }

    fn first(&self, parameters: &Parameters) -> Result<Option<Attributes>, Error> {
        let rows = self.query(parameters)?;
        Ok(rows.into_iter().next())
    }

    fn find(&self, id: i32, parameters: &Parameters) -> Result<Attributes, Error> {
        let (query, bindings) = QueryBuilder::new(self.table_schema).find(id, parameters)?;

        let rows = self.execute(query, bindings)?;
        let row = rows.into_iter().next().ok_or_else(|| Error::RecordNotFound)?;

        Ok(row)
    }

    fn insert(&self, attributes: Attributes, parameters: &Parameters) -> Result<Attributes, Error> {
        let (query, bindings) =
            QueryBuilder::new(self.table_schema).insert(attributes, parameters)?;
        let rows = self.execute(query, bindings)?;
        let row = rows.into_iter().next().ok_or_else(|| Error::RecordNotFound)?;

        Ok(row)
    }

    fn update(&self, id: i32, attributes: Attributes, parameters: &Parameters) -> Result<Attributes, Error> {
        let (query, bindings) = QueryBuilder::new(self.table_schema)
            .update(id, attributes, parameters)?;
        let rows = self.execute(query, bindings)?;
        let row = rows.into_iter().next().ok_or_else(|| Error::RecordNotFound)?;

        Ok(row)
    }

    fn delete(&self, id: i32) -> Result<(), Error> {
        let (query, bindings) = QueryBuilder::new(self.table_schema).delete(id);
        debug!("{}, {:?}", query, bindings);

        let bindings: Vec<&dyn ToSql> = bindings
            .iter()
            .map(|b| b as &dyn ToSql)
            .collect();
        self.connection.prepare(&query)?.execute(bindings.as_slice())?;
        Ok(())
    }

    fn execute(&self, query: String, bindings: Vec<Attribute>) -> Result<Vec<Attributes>, Error> {
        debug!("{}, {:?}", query, bindings);

        let bindings: Vec<&dyn ToSql> = bindings
            .iter()
            .map(|b| b as &dyn ToSql)
            .collect();
        let mut statement = self.connection.prepare(&query)?;
        let rows = statement
            .query_and_then(bindings.as_slice(), |row|
                attributes::materialise(&self.table_schema, row)
            )?
            .collect::<Result<Vec<Attributes>, _>>()?;

        debug!("Returned {} rows", rows.len());
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        attributes::Attribute,
        query::parameters::Parameters,
        schema::TableSchema,
    };
    use rusqlite::Connection;
    use std::error::Error as StdError;
    use tauri::http::Uri;

    fn mock_params(query: &str) -> Result<Parameters, Box<dyn StdError>> {
        let uri = format!("http://host.com/resource?{}", query).parse::<Uri>()?;
        Ok(Parameters::parse(&uri)?)
    }

    struct Context {
        connection: Connection,
        schema: TableSchema,
    }

    impl Context {
        pub(super) fn new() -> Self {
            let connection = Connection::open_in_memory().unwrap();
            let schema = TableSchema {
                name: "my_table",
                columns: &[
                    ("id", AttributeType::Integer),
                    ("col1", AttributeType::Text),
                    ("col2", AttributeType::Text),
                    ("col3", AttributeType::Integer),
                ],
                relationships: &[],
                text_index: true
            };

            connection.execute_batch(
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
                "
            ).unwrap();
            Context { connection, schema }
        }

        pub(super) fn table(&self) -> Table<'_> {
            Table { schema: &self.table_schema, connection: &self.connection }
        }

        pub(super) fn seed_table(&self) -> Result<Table<'_>, Error> {
            let table = self.table();

            table.insert(
                [
                    ("col1".to_string(), Attribute::Text("The quick brown fox".to_string())),
                    ("col2".to_string(), Attribute::Text("jumps over the lazy dog".to_string())),
                    ("col3".to_string(), Attribute::Integer(42))
                ].into(),
                &Parameters::default()
            )?;
            table.insert(
                [
                    ("col1".to_string(), Attribute::Text("The five boxing wizards".to_string())),
                    ("col2".to_string(), Attribute::Text("jump quickly".to_string())),
                    ("col3".to_string(), Attribute::Integer(1000))
                ].into(),
                &Parameters::default()
            )?;
            table.insert(
                [
                    ("col1".to_string(), Attribute::Text("Pack my box".to_string())),
                    ("col2".to_string(), Attribute::Text("with five dozen liquor jugs".to_string())),
                    ("col3".to_string(), Attribute::Integer(-1000))
                ].into(),
                &Parameters::default()
            )?;

            Ok(table)
        }
    }

    #[test]
    fn test_query_without_records() {
        let context = Context::new();
        let parameters = Parameters::default();
        let result = context.table().query(&parameters);

        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_first_without_records() {
        let context = Context::new();
        let parameters = Parameters::default();
        let result = context.table().first(&parameters);

        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_find_without_records() {
        let context = Context::new();
        let parameters = Parameters::default();
        let result = context.table().find(1, &parameters);

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), Error::RecordNotFound));
    }


    #[test]
    fn test_query() -> Result<(), Box<dyn StdError>> {
        let context = Context::new();
        let table = context.seed_table()?;

        let default_params_result = table.query(&Parameters::default())?;

        assert_eq!(default_params_result.len(), 3);

        let find_single_params = mock_params("filter[col1]=eq:The%20quick%20brown%20fox&filter[col3]=eq:42")?;
        let find_single_result = table.query(&find_single_params)?;

        assert_eq!(find_single_result.len(), 1);
        assert_eq!(find_single_result[0].get("col1").unwrap(), &Attribute::Text("The quick brown fox".to_string()));

        let find_many_params = mock_params("filter[col2]=like:jump")?;
        let find_many_result = table.query(&find_many_params)?;

        assert_eq!(find_many_result.len(), 2);
        assert_eq!(find_many_result[0].get("col3").unwrap(), &Attribute::Integer(42));
        assert_eq!(find_many_result[1].get("col3").unwrap(), &Attribute::Integer(1000));

        let find_none_params = mock_params("filter[col3]=lt:50&filter[col1]=like:I%20am%20not%20here")?;
        let find_none_result = table.query(&find_none_params)?;

        assert_eq!(find_none_result.len(), 0);

        let text_search_params = mock_params("search=five,box")?;
        let text_search_result = table.query(&text_search_params)?;

        assert_eq!(text_search_result.len(), 2);
        assert_eq!(text_search_result[0].get("col3").unwrap(), &Attribute::Integer(1000));
        assert_eq!(text_search_result[1].get("col3").unwrap(), &Attribute::Integer(-1000));

        Ok(())
    }

    #[test]
    fn test_first() -> Result<(), Box<dyn StdError>> {
        let context = Context::new();
        let table = context.seed_table()?;
        let result = table.first(&Parameters::default())?;

        assert!(result.is_some());
        assert_eq!(result.unwrap().get("col1"), Some(&Attribute::Text("The quick brown fox".to_string())));

        Ok(())
    }

    #[test]
    fn test_find() {
        let context = Context::new();
        let parameters = Parameters::default();
        let result = context.table().find(1, &parameters);

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), Error::RecordNotFound));
    }

    #[test]
    fn test_insert() {
        let context = Context::new();
        let attributes = Record::from([
            ("col1".to_string(), Attribute::Text("value1".to_string())),
        ]);
        let parameters = Parameters::default();
        let result = context.table().insert(attributes, &parameters);

        assert!(result.is_ok());
        let row = result.unwrap();
        assert_eq!(row.get("col1").unwrap(), &Attribute::Text("value1".to_string()));
    }

    #[test]
    fn test_update() {
        let context = Context::new();
        let insert_attributes = Record::from([
            ("col1".to_string(), Attribute::Text("value1".to_string())),
        ]);
        let insert_parameters = Parameters::default();
        context.table().insert(insert_attributes, &insert_parameters).unwrap();

        let update_attributes = Record::from([
            ("col1".to_string(), Attribute::Text("new_value".to_string())),
        ]);
        let update_parameters = Parameters::default();
        let result = context.table().update(1, update_attributes, &update_parameters);

        assert!(result.is_ok());
        let row = result.unwrap();
        assert_eq!(row.get("col1").unwrap(), &Attribute::Text("new_value".to_string()));
    }

    #[test]
    fn test_delete() {
        let context = Context::new();
        let attributes = Record::from([
            ("col1".to_string(), Attribute::Text("value1".to_string())),
        ]);
        let parameters = Parameters::default();
        context.table().insert(attributes, &parameters).unwrap();

        let result = context.table().delete(1);
        assert!(result.is_ok());

        let find_result = context.table().find(1, &parameters);
        assert!(find_result.is_err());
        assert!(matches!(find_result.unwrap_err(), Error::RecordNotFound));
    }

    #[test]
    fn test_execute() {
        let context = Context::new();
        let query = "SELECT * FROM my_table".to_string();
        let bindings = Vec::new();
        let result = context.table().execute(query, bindings);

        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_execute_with_bindings() {
        let context = Context::new();
        let query = "INSERT INTO my_table (col1) VALUES (?1) RETURNING col1".to_string();
        let bindings = vec![Attribute::Text("value1".to_string())];
        let result = context.table().execute(query, bindings);

        assert!(result.is_ok());
        let rows = result.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("col1").unwrap(), &Attribute::Text("value1".to_string()));
    }

    #[test]
    fn test_execute_with_invalid_query() {
        let context = Context::new();
        let query = "INVALID SQL".to_string();
        let bindings = Vec::new();
        let result = context.table().execute(query, bindings);

        assert!(result.is_err());
    }

    #[test]
    fn test_execute_with_invalid_bindings() {
        let context = Context::new();
        let query = "SELECT * FROM my_table WHERE id = ?1".to_string();
        let bindings = vec![];
        let result = context.table().execute(query, bindings);

        assert!(result.is_err());
    }
}