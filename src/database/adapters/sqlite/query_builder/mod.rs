#[cfg(test)]
mod tests;

use crate::database::attributes::Identifier;
use crate::database::{
    attributes::{Attribute, Attributes},
    error::Error,
    query_builder::QueryBuilder as QueryBuilderInterface,
    query_parameters::{
        FieldsParameters, FilterParameters, FilterValue, PageParameters, QueryParameters,
        SearchParameters, SortDirection, SortParameters, SortingAttribute,
    },
    schema::{AttributeType, Schema},
};
use indexmap::IndexSet;
use itertools::Itertools;

struct ExtractedAttributes<'sch> {
    fields: Vec<&'sch str>,
    values: Vec<Attribute>,
}

pub type Bindings = Vec<Attribute>;

/// Binds values for a parameterised query, returning the positional placeholder (`?N`) for each so
/// numbering stays consistent across every clause that contributes parameters.
trait Binder {
    fn bind(&mut self, value: Attribute) -> String;
    fn bind_all(&mut self, values: impl IntoIterator<Item = Attribute>) -> Vec<String>;
}

impl Binder for Bindings {
    fn bind(&mut self, value: Attribute) -> String {
        self.push(value);
        format!("?{}", self.len())
    }

    fn bind_all(&mut self, values: impl IntoIterator<Item = Attribute>) -> Vec<String> {
        values.into_iter().map(|value| self.bind(value)).collect()
    }
}

/// Whether any row can satisfy a built `WHERE` clause. An empty `IN` set makes a match
/// `Impossible` — it can match nothing — letting the caller skip the query and return no rows.
enum Match {
    Possible,
    Impossible,
}

pub struct QueryBuilder<'sch> {
    schema: &'sch Schema<'sch>,
}

impl<'sch> QueryBuilder<'sch> {
    fn build_select_clause(&self, fields: &FieldsParameters, query: &mut Vec<String>) {
        query.extend(["SELECT".to_string(), self.fields_for_model(fields, true)]);
    }

    fn build_insert_clause(
        &self,
        rows: Vec<Attributes<'sch>>,
        query: &mut Vec<String>,
        bindings: &mut Bindings,
    ) -> Result<(), Error> {
        let mut rows = rows.into_iter();
        let Some(first) = rows.next() else {
            return Err(Error::InvalidOperation {
                schema: self.schema.name().to_string(),
                operation: "INSERT".to_string(),
                message: "cannot insert without any records".to_string(),
            });
        };
        let columns: Vec<&str> = first.keys().copied().collect();

        let mut tuples = Vec::new();
        for mut row in std::iter::once(first).chain(rows) {
            let mut placeholders = Vec::with_capacity(columns.len());
            for column in &columns {
                let value = row
                    .swap_remove(column)
                    .ok_or_else(|| self.uniform_columns_error())?;
                placeholders.push(bindings.bind(value));
            }

            if !row.is_empty() {
                return Err(self.uniform_columns_error());
            }
            tuples.push(format!("({})", placeholders.join(", ")));
        }

        query.extend([
            "INSERT INTO".to_string(),
            format!("{}({})", self.schema.name(), columns.join(", ")),
            format!("VALUES {}", tuples.join(", ")),
        ]);

        Ok(())
    }

    fn uniform_columns_error(&self) -> Error {
        Error::InvalidOperation {
            schema: self.schema.name().to_string(),
            operation: "INSERT".to_string(),
            message: "every record in a batch must declare the same attributes".to_string(),
        }
    }

    fn build_update_clause(
        &self,
        attributes: Attributes<'sch>,
        query: &mut Vec<String>,
        bindings: &mut Bindings,
    ) {
        let attributes = self.extract_attributes(attributes);
        query.extend(["UPDATE".to_string(), self.schema.name().to_string()]);

        if attributes.fields.is_empty() {
            return;
        }

        let assignments = attributes
            .fields
            .into_iter()
            .zip(attributes.values)
            .map(|(field, value)| format!("{} = {}", field, bindings.bind(value)))
            .join(", ");
        query.extend(["SET".to_string(), assignments]);
    }

    fn build_from_clause(&self, query: &mut Vec<String>) {
        query.extend(["FROM".to_string(), self.schema.name().to_string()]);
    }

    fn build_join_clause(
        &self,
        search: &Option<SearchParameters>,
        query: &mut Vec<String>,
    ) -> Result<(), Error> {
        if search.is_none() {
            return Ok(());
        }

        if !self.schema.text_index() {
            return Err(Error::QueryValidationFailure {
                schema: self.schema.name().to_string(),
                attribute: "search".to_string(),
                message: "This resource does not support full-text search".to_string(),
            });
        }

        query.push(format!(
            "JOIN {}_fts fts ON {}.id = fts.rowid",
            self.schema.name(),
            self.schema.name()
        ));

        Ok(())
    }

    /// Renders the `WHERE` clause, reporting whether any row can match so the caller can skip an
    /// impossible query. An empty `IN` set matches no row (`x IN ()` is always false), making the
    /// whole query `Impossible`; an empty `NOT IN` set matches every row (always true) and is
    /// dropped. The `WHERE` keyword is emitted only when at least one predicate remains.
    fn build_where_clause(
        &self,
        filter: &Option<FilterParameters>,
        search: &Option<SearchParameters>,
        query: &mut Vec<String>,
        bindings: &mut Bindings,
    ) -> Result<Match, Error> {
        use FilterValue::*;

        if filter.is_none() && search.is_none() {
            return Ok(Match::Possible);
        }

        let mut filter_query = Vec::new();

        if let Some(values) = search {
            for value in values {
                filter_query.push(format!(
                    "{}_fts MATCH {}",
                    self.schema.name(),
                    bindings.bind(Attribute::Text(value.to_string()))
                ));
            }
        }

        if let Some(filter) = filter {
            for (field, filters) in filter {
                let table = self.schema.name();
                let kind = self
                    .schema
                    .column(field)
                    .ok_or_else(|| Error::InvalidAttributeAccess {
                        schema: self.schema.name().to_string(),
                        attribute: field.to_string(),
                    })?
                    .kind;

                for filter in filters {
                    match filter {
                        In(values) => {
                            if values.is_empty() {
                                return Ok(Match::Impossible);
                            }
                            let placeholders = bindings.bind_all(values.iter().cloned()).join(",");
                            filter_query.push(format!("{table}.{field} IN ({placeholders})"));
                        }
                        NotIn(values) => {
                            if values.is_empty() {
                                continue;
                            }
                            let placeholders = bindings.bind_all(values.iter().cloned()).join(",");
                            filter_query.push(format!("{table}.{field} NOT IN ({placeholders})"));
                        }
                        Like(value) => {
                            let binding = if matches!(kind, AttributeType::Text) {
                                Attribute::Text(format!("%{}%", value))
                            } else {
                                return Err(Error::QueryValidationFailure {
                                    schema: self.schema.name().to_string(),
                                    attribute: field.to_string(),
                                    message:
                                        "The 'LIKE' operator can only be applied to text attributes"
                                            .to_string(),
                                });
                            };
                            filter_query
                                .push(format!("{table}.{field} LIKE {}", bindings.bind(binding)));
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

                            filter_query.push(format!(
                                "{table}.{field} {operator} {}",
                                bindings.bind(binding.clone())
                            ));
                        }
                    }
                }
            }
        }

        if !filter_query.is_empty() {
            query.push("WHERE".to_string());
            query.push(filter_query.join(" AND "));
        }

        Ok(Match::Possible)
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
                sort_query.push(format!("{}.{} {}", self.schema.name(), field, direction));
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
        let rendered = fields
            .get(self.schema.name())
            .expect("Columns for all requested models should have been pre-loaded by the query parameters parser")
            .iter()
            .map(|field| {
                if self.schema.is_primary_key(field)
                    || self.schema.has_attribute(field)
                    || self.schema.has_foreign_key(field)
                {
                    field
                } else {
                    self.schema
                        .relationship(field)
                        .expect(
                            "\
                            All columns provided to the query builder should have been pre-validated \
                            by the query parameters parser\
                            "
                        )
                        .related
                        .keys.own
                }
            });

        let columns: IndexSet<&str> = std::iter::once(self.schema.primary_key().name)
            .chain(rendered)
            .collect();

        if qualified {
            columns
                .iter()
                .map(|column| format!("{}.{}", self.schema.name(), column))
                .join(", ")
        } else {
            columns.iter().join(", ")
        }
    }

    fn extract_attributes(&self, attributes: Attributes<'sch>) -> ExtractedAttributes<'sch> {
        let mut fields = Vec::<&'sch str>::new();
        let mut values = Vec::<Attribute>::new();

        for (field, value) in attributes {
            fields.push(field);
            values.push(value);
        }

        ExtractedAttributes { fields, values }
    }
}

impl<'sch> QueryBuilderInterface<'sch> for QueryBuilder<'sch> {
    fn new(schema: &'sch Schema<'sch>) -> Self {
        Self { schema }
    }

    fn query(&self, parameters: &QueryParameters) -> Result<Option<(String, Bindings)>, Error> {
        let mut query = Vec::new();
        let mut bindings = Bindings::new();

        self.build_select_clause(&parameters.fields, &mut query);
        self.build_from_clause(&mut query);
        self.build_join_clause(&parameters.search, &mut query)?;
        if let Match::Impossible = self.build_where_clause(
            &parameters.filter,
            &parameters.search,
            &mut query,
            &mut bindings,
        )? {
            return Ok(None);
        }
        self.build_order_by_clause(&parameters.sort, &mut query);
        self.build_limit_offset_clauses(&parameters.page, &mut query);

        Ok(Some((query.join(" "), bindings)))
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
        attributes: Attributes<'sch>,
        parameters: &QueryParameters,
    ) -> Result<(String, Bindings), Error> {
        let mut query = Vec::new();
        let mut bindings = Bindings::new();

        self.build_insert_clause(vec![attributes], &mut query, &mut bindings)?;
        self.build_returning_clause(&parameters.fields, &mut query);

        Ok((query.join(" "), bindings))
    }

    fn insert_batch(
        &self,
        rows: Vec<Attributes<'sch>>,
        parameters: &QueryParameters,
    ) -> Result<(String, Bindings), Error> {
        let mut query = Vec::new();
        let mut bindings = Bindings::new();

        self.build_insert_clause(rows, &mut query, &mut bindings)?;
        self.build_returning_clause(&parameters.fields, &mut query);

        Ok((query.join(" "), bindings))
    }

    fn update(
        &self,
        id: Identifier,
        attributes: Attributes<'sch>,
        parameters: &QueryParameters,
    ) -> Result<(String, Bindings), Error> {
        let mut query = Vec::new();
        let mut bindings = Bindings::new();

        self.build_update_clause(attributes, &mut query, &mut bindings);
        query.push(format!("WHERE id = {}", bindings.bind(Attribute::from(id))));
        self.build_returning_clause(&parameters.fields, &mut query);

        Ok((query.join(" "), bindings))
    }

    fn update_batch(
        &self,
        attributes: Attributes<'sch>,
        parameters: &QueryParameters,
    ) -> Result<Option<(String, Bindings)>, Error> {
        let mut query = Vec::new();
        let mut bindings = Bindings::new();

        self.build_update_clause(attributes, &mut query, &mut bindings);
        if let Match::Impossible =
            self.build_where_clause(&parameters.filter, &None, &mut query, &mut bindings)?
        {
            return Ok(None);
        }
        self.build_returning_clause(&parameters.fields, &mut query);

        Ok(Some((query.join(" "), bindings)))
    }

    fn delete(&self, id: Identifier) -> (String, Bindings) {
        (
            format!("DELETE FROM {} WHERE id = ?1", self.schema.name()),
            [Attribute::from(id)].into(),
        )
    }

    fn delete_batch(
        &self,
        parameters: &QueryParameters,
    ) -> Result<Option<(String, Bindings)>, Error> {
        let mut query = vec!["DELETE FROM".to_string(), self.schema.name().to_string()];
        let mut bindings = Bindings::new();

        if let Match::Impossible =
            self.build_where_clause(&parameters.filter, &None, &mut query, &mut bindings)?
        {
            return Ok(None);
        }

        Ok(Some((query.join(" "), bindings)))
    }
}
