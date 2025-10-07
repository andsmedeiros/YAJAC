use crate::database::{
    attributes::{Attribute, Record},
    error::Error,
    schema::TableSchema
};
use crate::routing::parameters::QueryParameters as Parameters;

pub struct ExtractedAttributes {
    fields: Vec<String>,
    values: Vec<Attribute>
}

impl ExtractedAttributes {
    pub(in crate::database) fn to_placeholders(&self) -> Vec<String> {
        (1..=self.fields.len())
            .into_iter()
            .map(|i| format!("?{i}"))
            .collect()
    }
}

pub type Bindings = Vec<Attribute>;

pub trait QueryBuilder {
    fn new(schema: &TableSchema) -> Self;
    fn query(&self, parameters: &Parameters) -> Result<(String, Bindings), Error>;
    fn find(&self, id: i32, parameters: &Parameters) -> Result<(String, Bindings), Error>;
    fn insert(&self, attributes: Record, parameters: &Parameters) -> Result<(String, Bindings), Error>;
    fn update(&self, id: i32, attributes: Record, parameters: &Parameters) -> Result<(String, Bindings), Error>;
    fn delete(&self, id: i32) -> (String, Bindings);
}

#[cfg(test)]
mod tests {
    use super::*;
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
}