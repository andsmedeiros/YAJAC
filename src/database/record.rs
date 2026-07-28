use super::{
    attributes::{Attributes, Identifier},
    error::Error,
    relationships::{Relationship, Relationships},
    schema::{IdentifierType, Schema},
};
use crate::database::attributes::{Attribute, ForeignKeys, Row};
use crate::json_api::identifier::Identifier as JsonApiIdentifier;

pub trait Builder<'sch>: From<(&'sch Schema<'sch>, Attributes<'sch>, Relationships<'sch>)> {
    fn new(schema: &'sch Schema<'sch>) -> Self;
    fn from_attributes(schema: &'sch Schema<'sch>, attributes: Attributes<'sch>) -> Self;
    fn from_relationships(schema: &'sch Schema<'sch>, relationships: Relationships<'sch>) -> Self;
}

#[derive(Debug, Clone)]
pub struct Record<'sch> {
    pub schema: &'sch Schema<'sch>,
    pub id: Option<Identifier>,
    pub attributes: Attributes<'sch>,
    pub relationships: Relationships<'sch>,
    pub(crate) foreign_keys: ForeignKeys<'sch>,
}

impl<'sch> Record<'sch> {
    pub fn with_id(mut self, id: Option<Identifier>) -> Self {
        self.id = id;
        self
    }

    pub fn with_attributes(mut self, attributes: Attributes<'sch>) -> Self {
        self.attributes = attributes;
        self
    }

    pub fn with_relationships(mut self, relationships: Relationships<'sch>) -> Self {
        self.relationships = relationships;
        self
    }

    pub fn kind(&self) -> &'sch str {
        self.schema.name()
    }

    pub fn schema(&self) -> &'sch Schema<'sch> {
        self.schema
    }

    pub fn identifier(&self) -> JsonApiIdentifier {
        self.id
            .as_ref()
            .map(|id| {
                let id = match id {
                    Identifier::Integer(value) => value.to_string(),
                    Identifier::Text(value) => value.clone(),
                };
                JsonApiIdentifier::Existing {
                    kind: self.kind().to_string(),
                    id,
                }
            })
            .unwrap_or_else(|| JsonApiIdentifier::New {
                kind: self.kind().to_string(),
                lid: None,
            })
    }

    pub fn get(&self, name: &str) -> Option<&Attribute> {
        self.attributes
            .get(name)
            .or_else(|| self.foreign_keys.get(name))
    }

    pub fn require(&self, name: &str) -> Result<&Attribute, Error> {
        self.get(name)
            .ok_or_else(|| Error::UnloadedAttributeAccess {
                schema: self.schema.name().to_string(),
                attribute: name.to_string(),
            })
    }

    pub fn get_id(&self) -> Option<&Identifier> {
        self.id.as_ref()
    }

    pub fn require_id(&self) -> Result<&Identifier, Error> {
        self.get_id().ok_or_else(|| Error::MissingRecordId {
            schema: self.schema.name().to_string(),
        })
    }

    pub fn get_owned(&self, name: &str) -> Option<Attribute> {
        if self.schema.is_primary_key(name) {
            self.get_id().cloned().map(Into::into)
        } else {
            self.get(name).cloned()
        }
    }

    pub fn require_owned(&self, name: &str) -> Result<Attribute, Error> {
        if self.schema.is_primary_key(name) {
            self.require_id().cloned().map(Into::into)
        } else {
            self.require(name).cloned()
        }
    }

    pub fn get_related(&self, relationship: &str) -> Option<&Relationship> {
        self.relationships.get(relationship)
    }

    pub fn require_related(&self, relationship: &str) -> Result<&Relationship, Error> {
        self.get_related(relationship)
            .ok_or_else(|| Error::UnloadedAttributeAccess {
                schema: self.schema.name().to_string(),
                attribute: relationship.to_string(),
            })
    }

    /// Synthesises a record from a `Table`-provided row, sorting its columns into the primary key,
    /// attributes and foreign keys declared by `schema`. The row's keys are already the schema's own
    /// `&'sch` names, so each is stored directly. The primary key is optional; any column the schema
    /// does not recognise is rejected.
    pub fn try_from_row(schema: &'sch Schema<'sch>, row: Row<'sch>) -> Result<Self, Error> {
        let mut id = None;
        let mut attributes = Attributes::new();
        let mut foreign_keys = ForeignKeys::new();

        for (name, value) in row {
            if schema.is_primary_key(name) {
                id = Some(match (schema.primary_key().kind, value) {
                    (IdentifierType::Integer, Attribute::Integer(value)) => {
                        Identifier::Integer(value)
                    }
                    (IdentifierType::Text, Attribute::Text(value)) => Identifier::Text(value),
                    (kind, value) => {
                        return Err(Error::InconsistentSchema {
                            schema: schema.name().to_string(),
                            attribute: schema.primary_key().name.to_string(),
                            message: format!(
                                "Expected primary key '{value:?}' to be of type '{kind}'"
                            ),
                        });
                    }
                });
            } else if schema.has_attribute(name) {
                attributes.insert(name, value);
            } else if schema.has_foreign_key(name) {
                foreign_keys.insert(name, value);
            } else {
                return Err(Error::InconsistentSchema {
                    schema: schema.name().to_string(),
                    attribute: name.to_string(),
                    message: "Database returned an unknown attribute".to_string(),
                });
            }
        }

        Ok(Record {
            schema,
            id,
            attributes,
            relationships: Relationships::new(),
            foreign_keys,
        })
    }

    /// Moves the columns out as a writable row, leaving the record column-less but with its id and
    /// relationships intact. Pair with `Refreshable::refresh_with` to refill from the persisted row.
    pub fn take_row(&mut self) -> Row<'sch> {
        let mut row = std::mem::take(&mut self.attributes);
        row.extend(std::mem::take(&mut self.foreign_keys));
        row
    }
}

impl<'sch> Builder<'sch> for Record<'sch> {
    fn new(schema: &'sch Schema<'sch>) -> Self {
        Record {
            schema,
            id: None,
            attributes: Attributes::new(),
            relationships: Relationships::new(),
            foreign_keys: ForeignKeys::new(),
        }
    }

    fn from_attributes(schema: &'sch Schema<'sch>, attributes: Attributes<'sch>) -> Self {
        Record {
            attributes,
            ..Self::new(schema)
        }
    }

    fn from_relationships(schema: &'sch Schema<'sch>, relationships: Relationships<'sch>) -> Self {
        Record {
            relationships,
            ..Self::new(schema)
        }
    }
}

impl<'sch> From<(&'sch Schema<'sch>, Attributes<'sch>, Relationships<'sch>)> for Record<'sch> {
    fn from(parts: (&'sch Schema<'sch>, Attributes<'sch>, Relationships<'sch>)) -> Self {
        let (schema, attributes, relationships) = parts;
        Record {
            attributes,
            relationships,
            ..Self::new(schema)
        }
    }
}

impl<'sch> From<RecordPatch<'sch>> for Record<'sch> {
    fn from(patch: RecordPatch<'sch>) -> Self {
        Record {
            schema: patch.schema,
            id: None,
            attributes: patch.attributes,
            relationships: patch.relationships,
            foreign_keys: ForeignKeys::new(),
        }
    }
}

impl<'sch> TryFrom<(&'sch Schema<'sch>, Row<'sch>)> for Record<'sch> {
    type Error = Error;

    fn try_from((schema, row): (&'sch Schema<'sch>, Row<'sch>)) -> Result<Self, Error> {
        Record::try_from_row(schema, row)
    }
}

/// Projects a record onto a flat row, carrying over its attributes and foreign keys. The primary
/// key (side-loaded on writes) and relationships (not columns) are dropped.
impl<'sch> From<Record<'sch>> for Row<'sch> {
    fn from(mut record: Record<'sch>) -> Self {
        record.take_row()
    }
}

/// Refreshes a record (or collection of records) from the row(s) a `producer` persists: the columns
/// are drained out, handed to `producer`, and the persisted columns it returns are written back,
/// leaving relationships untouched.
pub trait Refreshable {
    type Content;

    fn refresh_with(
        &mut self,
        producer: impl FnOnce(Self::Content) -> Result<Self::Content, Error>,
    ) -> Result<&mut Self, Error>;
}

impl<'sch> Refreshable for Record<'sch> {
    type Content = Row<'sch>;

    fn refresh_with(
        &mut self,
        producer: impl FnOnce(Row<'sch>) -> Result<Row<'sch>, Error>,
    ) -> Result<&mut Self, Error> {
        let row = producer(self.take_row())?;
        let refreshed = Record::try_from_row(self.schema, row)?;
        (self.id, self.attributes, self.foreign_keys) =
            (refreshed.id, refreshed.attributes, refreshed.foreign_keys);
        Ok(self)
    }
}

impl<'sch> Refreshable for Vec<Record<'sch>> {
    type Content = Vec<Row<'sch>>;

    fn refresh_with(
        &mut self,
        producer: impl FnOnce(Vec<Row<'sch>>) -> Result<Vec<Row<'sch>>, Error>,
    ) -> Result<&mut Self, Error> {
        let rows = producer(self.iter_mut().map(Record::take_row).collect())?;
        if rows.len() != self.len() {
            return Err(Error::InconsistentCollection);
        }
        for (record, row) in self.iter_mut().zip(rows) {
            let refreshed = Record::try_from_row(record.schema, row)?;
            (record.id, record.attributes, record.foreign_keys) =
                (refreshed.id, refreshed.attributes, refreshed.foreign_keys);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone)]
pub struct RecordPatch<'sch> {
    pub schema: &'sch Schema<'sch>,
    pub attributes: Attributes<'sch>,
    pub relationships: Relationships<'sch>,
}

impl<'sch> Builder<'sch> for RecordPatch<'sch> {
    fn new(schema: &'sch Schema<'sch>) -> Self {
        Self {
            schema,
            attributes: Attributes::new(),
            relationships: Relationships::new(),
        }
    }

    fn from_attributes(schema: &'sch Schema<'sch>, attributes: Attributes<'sch>) -> Self {
        Self {
            attributes,
            ..Self::new(schema)
        }
    }

    fn from_relationships(schema: &'sch Schema<'sch>, relationships: Relationships<'sch>) -> Self {
        RecordPatch {
            relationships,
            ..Self::new(schema)
        }
    }
}

impl<'sch> From<(&'sch Schema<'sch>, Attributes<'sch>, Relationships<'sch>)> for RecordPatch<'sch> {
    fn from(parts: (&'sch Schema<'sch>, Attributes<'sch>, Relationships<'sch>)) -> Self {
        let (schema, attributes, relationships) = parts;
        RecordPatch {
            attributes,
            relationships,
            ..Self::new(schema)
        }
    }
}
