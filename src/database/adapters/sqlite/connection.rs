use log::debug;
use rusqlite::{Connection, Result, Statement, ToSql};
use crate::database::{
    attributes,
    attributes::{Attribute, Attributes},
    connection::Connection as ConnectionInterface,
    error::Error
};

fn build_bindings(bindings: &Vec<Attribute>) -> Vec<&dyn ToSql> {
    bindings
        .iter()
        .map(|b| b as &dyn ToSql)
        .collect()
}

impl ConnectionInterface for Connection {
    fn query(&mut self, query: String, bindings: Vec<Attribute>) -> Result<Vec<Attributes>, Error> {
        debug!("{}, {:?}", query, bindings);

        let bindings = build_bindings(&bindings);
        let mut statement = self.prepare(&query)?;
        let rows = statement
            .query_and_then(bindings.as_slice(), |row|
                attributes::materialise(&self.table_schema, row)
            )?
            .collect::<Result<Vec<Attributes>, _>>()?;

        debug!("Returned {} rows", rows.len());
        Ok(rows)
    }

    fn execute(&mut self, query: String, bindings: Vec<Attribute>) -> Result<(), Error> {
        debug!("{}, {:?}", query, bindings);

        let bindings = build_bindings(&bindings);
        let mut statement = self.prepare(&query)?;
        let row_count = statement.execute(&bindings)?;

        debug!("Affected {} rows", row_count);
        Ok(())
    }
}
