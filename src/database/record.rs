use itertools::Itertools;

use super::{
    attributes::{Attributes, Identifier},
    error::Error,
    relationships::{Relationship, Relationships},
    schema::{IdentifierType, TableSchema},
};
use crate::database::attributes::{Attribute, ForeignKeys, Row};
use crate::json_api::identifier::Identifier as JsonApiIdentifier;
use std::{borrow::Borrow, collections::HashMap};

pub trait Builder<'sch>: From<(&'sch TableSchema<'sch>, Attributes, Relationships<'sch>)> {
    fn new(schema: &'sch TableSchema<'sch>) -> Self;
    fn from_attributes(schema: &'sch TableSchema<'sch>, attributes: Attributes) -> Self;
    fn from_relationships(
        schema: &'sch TableSchema<'sch>,
        relationships: Relationships<'sch>,
    ) -> Self;
}

#[derive(Debug, Clone)]
pub struct Record<'sch> {
    pub schema: &'sch TableSchema<'sch>,
    pub id: Option<Identifier>,
    pub attributes: Attributes,
    pub relationships: Relationships<'sch>,
    pub(crate) foreign_keys: ForeignKeys<'sch>,
}

impl<'sch> Record<'sch> {
    pub fn with_id(mut self, id: Option<Identifier>) -> Self {
        self.id = id;
        self
    }

    pub fn with_attributes(mut self, attributes: Attributes) -> Self {
        self.attributes = attributes;
        self
    }

    pub fn with_relationships(mut self, relationships: Relationships<'sch>) -> Self {
        self.relationships = relationships;
        self
    }

    pub fn kind(&self) -> &'sch str {
        self.schema.name
    }

    pub fn schema(&self) -> &'sch TableSchema<'sch> {
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
                schema: self.schema.name.to_string(),
                attribute: name.to_string(),
            })
    }

    pub fn get_id(&self) -> Option<&Identifier> {
        self.id.as_ref()
    }

    pub fn require_id(&self) -> Result<&Identifier, Error> {
        self.get_id().ok_or_else(|| Error::MissingRecordId {
            schema: self.schema.name.to_string(),
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
                schema: self.schema.name.to_string(),
                attribute: relationship.to_string(),
            })
    }

    /// Synthesises a record from a `Table`-provided row, sorting its columns into the primary key,
    /// attributes and foreign keys declared by `schema`. The primary key is optional; any column
    /// the schema does not recognise is rejected.
    pub fn try_from_row(schema: &'sch TableSchema<'sch>, row: Row) -> Result<Self, Error> {
        let mut id = None;
        let mut attributes = Attributes::new();
        let mut foreign_keys = ForeignKeys::new();

        for (name, value) in row {
            if schema.is_primary_key(&name) {
                id = Some(match (schema.primary_key.kind, value) {
                    (IdentifierType::Integer, Attribute::Integer(value)) => {
                        Identifier::Integer(value)
                    }
                    (IdentifierType::Text, Attribute::Text(value)) => Identifier::Text(value),
                    (kind, value) => {
                        return Err(Error::SchemaValidationFailure {
                            schema: schema.name.to_string(),
                            attribute: schema.primary_key.name.to_string(),
                            message: format!(
                                "Expected primary key '{value:?}' to be of type '{kind}'"
                            ),
                        });
                    }
                });
            } else if schema.has_attribute(&name) {
                attributes.insert(name, value);
            } else if schema.has_foreign_key(&name) {
                let (key, _) = schema
                    .foreign_keys
                    .iter()
                    .find(|(key, _)| **key == *name)
                    .expect("has_foreign_key guarantees the column is present");
                foreign_keys.insert(*key, value);
            } else {
                return Err(Error::SchemaValidationFailure {
                    schema: schema.name.to_string(),
                    attribute: name,
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
    pub fn take_row(&mut self) -> Row {
        let mut row = std::mem::take(&mut self.attributes);
        row.extend(
            std::mem::take(&mut self.foreign_keys)
                .into_iter()
                .map(|(key, value)| (key.to_string(), value)),
        );
        row
    }
}

impl<'sch> Builder<'sch> for Record<'sch> {
    fn new(schema: &'sch TableSchema<'sch>) -> Self {
        Record {
            schema,
            id: None,
            attributes: Attributes::new(),
            relationships: Relationships::new(),
            foreign_keys: ForeignKeys::new(),
        }
    }

    fn from_attributes(schema: &'sch TableSchema<'sch>, attributes: Attributes) -> Self {
        Record {
            attributes,
            ..Self::new(schema)
        }
    }

    fn from_relationships(
        schema: &'sch TableSchema<'sch>,
        relationships: Relationships<'sch>,
    ) -> Self {
        Record {
            relationships,
            ..Self::new(schema)
        }
    }
}

impl<'sch> From<(&'sch TableSchema<'sch>, Attributes, Relationships<'sch>)> for Record<'sch> {
    fn from(parts: (&'sch TableSchema<'sch>, Attributes, Relationships<'sch>)) -> Self {
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

impl<'sch> TryFrom<(&'sch TableSchema<'sch>, Row)> for Record<'sch> {
    type Error = Error;

    fn try_from((schema, row): (&'sch TableSchema<'sch>, Row)) -> Result<Self, Error> {
        Record::try_from_row(schema, row)
    }
}

/// Projects a record onto a flat row, carrying over its attributes and foreign keys. The primary
/// key (side-loaded on writes) and relationships (not columns) are dropped.
impl<'sch> From<Record<'sch>> for Row {
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
    ) -> Result<(), Error>;
}

impl<'sch> Refreshable for Record<'sch> {
    type Content = Row;

    fn refresh_with(
        &mut self,
        producer: impl FnOnce(Row) -> Result<Row, Error>,
    ) -> Result<(), Error> {
        let row = producer(self.take_row())?;
        let refreshed = Record::try_from_row(self.schema, row)?;
        (self.id, self.attributes, self.foreign_keys) =
            (refreshed.id, refreshed.attributes, refreshed.foreign_keys);
        Ok(())
    }
}

impl<'sch> Refreshable for Vec<Record<'sch>> {
    type Content = Vec<Row>;

    fn refresh_with(
        &mut self,
        producer: impl FnOnce(Vec<Row>) -> Result<Vec<Row>, Error>,
    ) -> Result<(), Error> {
        let rows = producer(self.iter_mut().map(Record::take_row).collect())?;
        if rows.len() != self.len() {
            return Err(Error::InconsistentCollection);
        }
        for (record, row) in self.iter_mut().zip(rows) {
            let refreshed = Record::try_from_row(record.schema, row)?;
            (record.id, record.attributes, record.foreign_keys) =
                (refreshed.id, refreshed.attributes, refreshed.foreign_keys);
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct RecordPatch<'sch> {
    pub schema: &'sch TableSchema<'sch>,
    pub attributes: Attributes,
    pub relationships: Relationships<'sch>,
}

impl<'sch> Builder<'sch> for RecordPatch<'sch> {
    fn new(schema: &'sch TableSchema<'sch>) -> Self {
        Self {
            schema,
            attributes: Attributes::new(),
            relationships: Relationships::new(),
        }
    }

    fn from_attributes(schema: &'sch TableSchema<'sch>, attributes: Attributes) -> Self {
        Self {
            attributes,
            ..Self::new(schema)
        }
    }

    fn from_relationships(
        schema: &'sch TableSchema<'sch>,
        relationships: Relationships<'sch>,
    ) -> Self {
        RecordPatch {
            relationships,
            ..Self::new(schema)
        }
    }
}

impl<'sch> From<(&'sch TableSchema<'sch>, Attributes, Relationships<'sch>)> for RecordPatch<'sch> {
    fn from(parts: (&'sch TableSchema<'sch>, Attributes, Relationships<'sch>)) -> Self {
        let (schema, attributes, relationships) = parts;
        RecordPatch {
            attributes,
            relationships,
            ..Self::new(schema)
        }
    }
}

pub(crate) struct Index<'sch> {
    records: HashMap<Identifier, Record<'sch>>,
}

impl<'sch> Index<'sch> {
    pub fn try_from_iter(iter: impl Iterator<Item = Record<'sch>>) -> Result<Self, Error> {
        let index = Self {
            records: iter
                .map(|record| -> Result<_, Error> { Ok((record.require_id()?.clone(), record)) })
                .try_collect()?,
        };

        Ok(index)
    }

    pub fn get(&self, id: impl Borrow<Identifier>) -> Option<&Record<'sch>> {
        self.records.get(id.borrow())
    }

    pub fn get_mut(&mut self, id: impl Borrow<Identifier>) -> Option<&mut Record<'sch>> {
        self.records.get_mut(id.borrow())
    }

    pub fn require(&self, id: impl Borrow<Identifier>) -> Result<&Record<'sch>, Error> {
        self.get(id).ok_or(Error::InvalidIndexAccess)
    }

    pub fn require_mut(&mut self, id: impl Borrow<Identifier>) -> Result<&mut Record<'sch>, Error> {
        self.get_mut(id).ok_or(Error::InvalidIndexAccess)
    }
}

pub(crate) type TableCache<'sch> = HashMap<&'sch str, Index<'sch>>;

pub(crate) trait Indexable<'sch: 'req, 'req> {
    fn index_by_primary_key(self) -> Result<Index<'sch>, Error>;
}

pub(crate) trait Groupable<'sch: 'req, 'req> {
    fn group_by(
        self,
        column: &str,
    ) -> Result<HashMap<&'req Attribute, Vec<&'req Record<'sch>>>, Error>;
}

impl<'sch: 'req, 'req, T> Indexable<'sch, 'req> for T
where
    T: Iterator<Item = Record<'sch>>,
{
    fn index_by_primary_key(self) -> Result<Index<'sch>, Error> {
        Index::try_from_iter(self)
    }
}

impl<'sch: 'req, 'req, T> Groupable<'sch, 'req> for T
where
    T: Iterator<Item = &'req Record<'sch>>,
{
    fn group_by(
        self,
        column: &str,
    ) -> Result<HashMap<&'req Attribute, Vec<&'req Record<'sch>>>, Error> {
        group_by(self, column)
    }
}

fn index_by<'sch, 'req>(
    records: impl Iterator<Item = &'req Record<'sch>>,
    column: &str,
) -> Result<HashMap<&'req Attribute, &'req Record<'sch>>, Error> {
    Ok(HashMap::from_iter(
        records
            .map(|record| -> Result<(&Attribute, &Record<'sch>), Error> {
                Ok((
                    record
                        .attributes
                        .get(column)
                        .or_else(|| record.foreign_keys.get(column))
                        .ok_or_else(|| Error::UnloadedAttributeAccess {
                            schema: record.schema.name.to_string(),
                            attribute: column.to_string(),
                        })?,
                    record,
                ))
            })
            .collect::<Result<Vec<_>, _>>()?,
    ))
}

fn group_by<'sch, 'req>(
    records: impl Iterator<Item = &'req Record<'sch>>,
    column: &str,
) -> Result<HashMap<&'req Attribute, Vec<&'req Record<'sch>>>, Error> {
    records
        .map(|record| {
            Ok((
                record
                    .attributes
                    .get(column)
                    .or_else(|| record.foreign_keys.get(column))
                    .ok_or_else(|| Error::UnloadedAttributeAccess {
                        schema: record.schema.name.to_string(),
                        attribute: column.to_string(),
                    })?,
                record,
            ))
        })
        .fold_ok(HashMap::new(), |mut groups, (attribute, record)| {
            groups.entry(attribute).or_default().push(record);
            groups
        })
}
