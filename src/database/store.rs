use core::slice;
use std::collections::HashMap;

use crate::database::adapters::Adapter as AdapterInterface;
use crate::database::attributes::{Attribute, Identifier, Row};
use crate::database::composite::{Composite, CompositeCollection, CompositeRecord};
use crate::database::connection::Connection as ConnectionInterface;
use crate::database::data_loader::DataLoader;
use crate::database::error::Error;
use crate::database::query_parameters::{FilterParameters, FilterValue, QueryParameters};
use crate::database::record::{Indexable, Record, RecordPatch, Refreshable};
use crate::database::registry::Registry;
use crate::database::relationships::Relationship as DatabaseRelationship;
use crate::database::schema::{Relationship as SchemaRelationship, TableSchema};
use crate::database::table::Table as TableInterface;
use indexmap::{IndexMap, IndexSet};
use itertools::Itertools;

pub struct Store<'sch, 'req, Adapter: AdapterInterface> {
    registry: &'sch Registry<'sch, Adapter>,
    connection: &'req Adapter::Connection,
}

impl<'sch, 'req, Adapter: AdapterInterface> Store<'sch, 'req, Adapter> {
    pub fn new(
        registry: &'sch Registry<'sch, Adapter>,
        connection: &'req Adapter::Connection,
    ) -> Self {
        Store {
            registry,
            connection,
        }
    }

    pub fn fetch_record(
        &self,
        schema: &'sch TableSchema<'sch>,
        id: Identifier,
        parameters: &QueryParameters<'sch, 'req>,
    ) -> Result<CompositeRecord<'sch>, Error> {
        self.connection.transaction(|| {
            let row = self.table(schema)?.find(id, parameters)?;
            let mut content = Record::try_from_row(schema, row)?;
            let included = self.loader().load_for_record(&mut content, parameters)?;

            Ok(Composite { content, included })
        })
    }

    pub fn fetch_collection(
        &self,
        schema: &'sch TableSchema<'sch>,
        parameters: &QueryParameters<'sch, 'req>,
    ) -> Result<CompositeCollection<'sch>, Error> {
        self.connection.transaction(|| {
            let mut content = self
                .table(schema)?
                .query(parameters)?
                .into_iter()
                .map(|row| Record::try_from_row(schema, row))
                .collect::<Result<Vec<_>, _>>()?;
            let included = self
                .loader()
                .load_for_collection(&mut content, parameters)?;

            Ok(Composite { content, included })
        })
    }

    /// Populates each record's `foreign_keys` with whatever `belongs_to` relationships
    /// are specified.
    /// This prepares the records for inserting or updating and must be called prior to any
    /// of these operations.
    fn attach_belongs_to(&self, records: &mut [Record<'sch>]) -> Result<(), Error> {
        let mut required_queries = HashMap::new();

        for record in records.iter_mut() {
            let schema = record.schema();
            for (&name, linkage) in &record.relationships {
                let &(name, ref descriptor) = schema
                    .relationships
                    .iter()
                    .find(|&entry| entry.0 == name)
                    .ok_or_else(|| Error::ResourceValidationFailure {
                        schema: schema.name.to_string(),
                        attribute: name.to_string(),
                        message: "Attempted to attach unknown relationship".to_string(),
                    })?;

                if let SchemaRelationship::BelongsTo(descriptor) = descriptor {
                    match linkage {
                        DatabaseRelationship::BelongsTo(id) => {
                            let related_table = self.registry.schema(descriptor.resource)?;
                            if related_table.is_primary_key(descriptor.keys.related) {
                                record
                                    .foreign_keys
                                    .insert(descriptor.keys.own, id.clone().into());
                            } else {
                                let (_, attributes, ids, relationships) = required_queries
                                    .entry(related_table.name)
                                    .or_insert_with(|| {
                                        (
                                            related_table,
                                            IndexSet::new(),
                                            IndexSet::new(),
                                            HashMap::new(),
                                        )
                                    });
                                attributes.insert(descriptor.keys.related);
                                ids.insert(id.clone().into());
                                relationships.insert(name, descriptor);
                            }
                        }
                        DatabaseRelationship::Empty => {
                            record
                                .foreign_keys
                                .insert(descriptor.keys.own, Attribute::Null);
                        }
                        _ => {
                            return Err(Error::ResourceValidationFailure {
                                schema: schema.name.to_string(),
                                attribute: name.to_string(),
                                message: "Attempted to attach relationship with wrong linkage"
                                    .to_string(),
                            });
                        }
                    }
                }
            }
        }

        for (related_table, attributes, ids, relationships) in required_queries.into_values() {
            let index = self
                .table(related_table)?
                .query(&QueryParameters {
                    fields: IndexMap::from([(related_table.name, attributes)]),
                    filter: Some(FilterParameters::from([(
                        related_table.primary_key.name,
                        vec![FilterValue::In(ids)],
                    )])),
                    ..QueryParameters::new(related_table)
                })?
                .into_iter()
                .map(|row| Record::try_from_row(related_table, row))
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .index_by_primary_key()?;

            for (relationship, descriptor) in relationships {
                for record in records.iter_mut() {
                    if let Some(DatabaseRelationship::BelongsTo(id)) =
                        record.relationships.get(relationship)
                    {
                        let related_record =
                            index.get(id).ok_or_else(|| Error::RelatedRecordNotFound {
                                relationship: relationship.to_string(),
                                resource: descriptor.resource.to_string(),
                                id: id.to_string(),
                            })?;
                        let value = related_record.require(descriptor.keys.related).cloned()?;
                        record.foreign_keys.insert(descriptor.keys.own, value);
                    }
                }
            }
        }

        Ok(())
    }

    fn attach_has_one_many(&self, records: &[Record<'sch>], replace: bool) -> Result<(), Error> {
        use DatabaseRelationship as Data;
        use SchemaRelationship as Schema;
        let mut patches: HashMap<&str, HashMap<Attribute, Row>> = HashMap::new();
        let mut full_detachments: HashMap<&str, HashMap<&str, IndexSet<_>>> = HashMap::new();

        for record in records.iter() {
            let schema = record.schema;
            for (name, relationship) in &record.relationships {
                let descriptor =
                    schema
                        .relationship(name)
                        .ok_or_else(|| Error::ResourceValidationFailure {
                            schema: schema.name.to_string(),
                            attribute: name.to_string(),
                            message: "Attempted to attach unknown relationship".to_string(),
                        })?;

                let (ids, descriptor) = match (&relationship, descriptor) {
                    (Data::Empty, Schema::HasOne(descriptor) | Schema::HasMany(descriptor)) => {
                        ([].as_slice(), descriptor)
                    }
                    (Data::HasOne(id), Schema::HasOne(descriptor)) => {
                        (slice::from_ref(id), descriptor)
                    }
                    (Data::HasMany(ids), Schema::HasMany(descriptor)) => {
                        (ids.as_slice(), descriptor)
                    }
                    (Data::HasOne(..) | Data::HasMany(..), _) => {
                        Err(Error::ResourceValidationFailure {
                            schema: schema.name.to_string(),
                            attribute: name.to_string(),
                            message: "Attempted to attach relationship with wrong linkage"
                                .to_string(),
                        })?
                    }
                    _ => continue,
                };

                let value = record.require_owned(descriptor.keys.own)?;
                if !ids.is_empty() {
                    for id in ids {
                        patches
                            .entry(descriptor.resource)
                            .or_default()
                            .entry(id.clone().into())
                            .or_default()
                            .insert(descriptor.keys.related.to_string(), value.clone());
                    }
                } else {
                    full_detachments
                        .entry(descriptor.resource)
                        .or_default()
                        .entry(descriptor.keys.related)
                        .or_default()
                        .insert(value);
                }
            }
        }

        let queries = patches.into_iter().map(|(table, patches)| {
            (
                table,
                patches.into_iter().fold(
                    HashMap::new(),
                    |mut map: HashMap<Vec<_>, IndexSet<_>>, (id, patch)| {
                        let key = patch
                            .into_iter()
                            .sorted_by(|a, b| Ord::cmp(a.0.as_str(), b.0.as_str()))
                            .collect_vec();
                        map.entry(key).or_default().insert(id);
                        map
                    },
                ),
            )
        });

        for (name, patches) in queries {
            let schema = self.registry.schema(name)?;
            let table = self.table(schema)?;
            for (patch, ids) in &patches {
                table.update_batch(
                    Row::from_iter(patch.clone()),
                    &QueryParameters {
                        filter: Some(FilterParameters::from([(
                            schema.primary_key.name,
                            vec![FilterValue::In(ids.clone())],
                        )])),
                        ..QueryParameters::new(schema)
                    },
                )?;
            }
            if replace {
                let patches = patches.into_iter().fold(
                    HashMap::new(),
                    |mut patches,
                     (attributes, ids)|
                     -> HashMap<(String, Attribute), IndexSet<Attribute>> {
                        for attribute in attributes {
                            patches.entry(attribute).or_default().extend(ids.clone())
                        }
                        patches
                    },
                );

                for ((name, value), ids) in patches {
                    table.update_batch(
                        Row::from([(name.clone(), Attribute::Null)]),
                        &QueryParameters {
                            filter: Some(FilterParameters::from([
                                (name.as_str(), vec![FilterValue::Equal(value)]),
                                (schema.primary_key.name, vec![FilterValue::NotIn(ids)]),
                            ])),
                            ..QueryParameters::new(schema)
                        },
                    )?;
                }
            }
        }

        if replace {
            for (schema, columns) in full_detachments {
                let schema = self.registry.schema(schema)?;
                let table = self.table(schema)?;

                for (column, values) in columns {
                    table.update_batch(
                        Row::from([(column.to_string(), Attribute::Null)]),
                        &QueryParameters {
                            filter: Some(
                                [(column, vec![FilterValue::In(values)])]
                                    .into_iter()
                                    .collect(),
                            ),
                            ..QueryParameters::new(schema)
                        },
                    )?;
                }
            }
        }

        Ok(())
    }

    pub fn create_record(
        &self,
        mut record: Record<'sch>,
        parameters: &QueryParameters<'sch, 'req>,
    ) -> Result<CompositeRecord<'sch>, Error> {
        self.connection.transaction(|| {
            let schema = record.schema;
            self.attach_belongs_to(slice::from_mut(&mut record))?;
            record.refresh_with(|row| self.table(schema)?.insert(row, parameters))?;
            self.attach_has_one_many(slice::from_ref(&record), false)?;
            let included = self.loader().load_for_record(&mut record, parameters)?;

            Ok(Composite {
                content: record,
                included,
            })
        })
    }

    pub fn update_record(
        &self,
        mut record: Record<'sch>,
        parameters: &QueryParameters<'sch, 'req>,
    ) -> Result<CompositeRecord<'sch>, Error> {
        self.connection.transaction(|| {
            let schema = record.schema;
            self.attach_belongs_to(slice::from_mut(&mut record))?;
            let id = record.require_id()?.clone();
            record.refresh_with(|row| {
                if row.is_empty() {
                    self.table(schema)?.find(id, parameters)
                } else {
                    self.table(schema)?.update(id, row, parameters)
                }
            })?;
            self.attach_has_one_many(slice::from_ref(&record), true)?;
            let included = self.loader().load_for_record(&mut record, parameters)?;

            Ok(Composite {
                content: record,
                included,
            })
        })
    }

    pub fn delete_record(
        &self,
        schema: &'sch TableSchema<'sch>,
        id: Identifier,
    ) -> Result<(), Error> {
        self.connection
            .transaction(|| self.table(schema)?.delete(id))
    }

    pub fn create_collection(
        &self,
        mut records: Vec<Record<'sch>>,
        parameters: &QueryParameters<'sch, 'req>,
    ) -> Result<CompositeCollection<'sch>, Error> {
        let schema = if let Some(first) = records.first() {
            first.schema
        } else {
            return Ok(Composite {
                content: Vec::new(),
                included: Vec::new(),
            });
        };

        if records.iter().any(|record| record.schema != schema) {
            return Err(Error::InconsistentCollection);
        }

        self.connection.transaction(|| {
            self.attach_belongs_to(&mut records)?;
            records.refresh_with(|rows| self.table(schema)?.insert_batch(rows, parameters))?;
            self.attach_has_one_many(&records, false)?;
            let included = self
                .loader()
                .load_for_collection(&mut records, parameters)?;

            Ok(Composite {
                content: records,
                included,
            })
        })
    }

    pub fn update_collection(
        &self,
        patch: RecordPatch<'sch>,
        parameters: &QueryParameters<'sch, 'req>,
    ) -> Result<CompositeCollection<'sch>, Error> {
        let schema = patch.schema;
        let mut patch = Record::from(patch);
        self.connection.transaction(|| {
            self.attach_belongs_to(slice::from_mut(&mut patch))?;
            let row = patch.take_row();
            let mut records = self
                .table(schema)?
                .update_batch(row, parameters)?
                .into_iter()
                .map(|row| {
                    Record::try_from_row(schema, row)
                        .map(|record| record.with_relationships(patch.relationships.clone()))
                })
                .collect::<Result<Vec<_>, _>>()?;
            self.attach_has_one_many(&records, true)?;
            let included = self
                .loader()
                .load_for_collection(&mut records, parameters)?;

            Ok(Composite {
                content: records,
                included,
            })
        })
    }

    pub fn delete_collection(
        &self,
        schema: &'sch TableSchema<'sch>,
        parameters: &QueryParameters<'sch, 'req>,
    ) -> Result<(), Error> {
        self.connection
            .transaction(|| self.table(schema)?.delete_batch(parameters))
    }

    fn table(&self, schema: &'sch TableSchema<'sch>) -> Result<Adapter::Table<'sch, 'req>, Error> {
        self.registry.table(schema.name, self.connection)
    }

    fn loader(&self) -> DataLoader<'sch, 'req, Adapter> {
        DataLoader::new(self.registry, self.connection)
    }
}

#[cfg(test)]
mod tests {
    use itertools::Itertools;

    use super::Store;
    use crate::database::adapters::SqliteAdapter;
    use crate::database::adapters::sqlite::{Connection, Pool};
    use crate::database::attributes::{Attribute, Attributes, Identifier, Row};
    use crate::database::error::Error;
    use crate::database::query_parameters::{FilterParameters, FilterValue, QueryParameters};
    use crate::database::record::{Builder, Record, RecordPatch};
    use crate::database::registry::Registry;
    use crate::database::relationships::{Relationship, Relationships};
    use crate::database::schema::{
        AttributeType, IdentifierType, PrimaryKey, RelatedResource,
        Relationship as SchemaRelationship, RelationshipKeys, TableSchema,
    };
    use crate::database::table::Table;
    use crate::http_wrappers::Uri;
    use std::collections::HashMap;
    use std::error::Error as StdError;
    use test_log::test;

    static USERS_SCHEMA: TableSchema = TableSchema {
        name: "users",
        primary_key: PrimaryKey {
            name: "id",
            kind: IdentifierType::Integer,
        },
        attributes: &[("name", AttributeType::Text)],
        foreign_keys: &[],
        relationships: &[
            (
                "posts",
                SchemaRelationship::HasMany(RelatedResource {
                    resource: "posts",
                    keys: RelationshipKeys {
                        related: "author_id",
                        own: "id",
                    },
                }),
            ),
            (
                "profile",
                SchemaRelationship::HasOne(RelatedResource {
                    resource: "profiles",
                    keys: RelationshipKeys {
                        related: "user_id",
                        own: "id",
                    },
                }),
            ),
        ],
        text_index: false,
    };

    static POSTS_SCHEMA: TableSchema = TableSchema {
        name: "posts",
        primary_key: PrimaryKey {
            name: "id",
            kind: IdentifierType::Integer,
        },
        attributes: &[("title", AttributeType::Text)],
        foreign_keys: &[("author_id", AttributeType::Integer)],
        relationships: &[(
            "author",
            SchemaRelationship::BelongsTo(RelatedResource {
                resource: "users",
                keys: RelationshipKeys {
                    related: "id",
                    own: "author_id",
                },
            }),
        )],
        text_index: false,
    };

    static PROFILES_SCHEMA: TableSchema = TableSchema {
        name: "profiles",
        primary_key: PrimaryKey {
            name: "id",
            kind: IdentifierType::Integer,
        },
        attributes: &[("bio", AttributeType::Text)],
        foreign_keys: &[("user_id", AttributeType::Integer)],
        relationships: &[(
            "user",
            SchemaRelationship::BelongsTo(RelatedResource {
                resource: "users",
                keys: RelationshipKeys {
                    related: "id",
                    own: "user_id",
                },
            }),
        )],
        text_index: false,
    };

    static SCHEMAS: [&TableSchema; 3] = [&USERS_SCHEMA, &POSTS_SCHEMA, &PROFILES_SCHEMA];

    fn with_registry<F>(func: F) -> Result<(), Box<dyn StdError>>
    where
        F: FnOnce(&Registry<SqliteAdapter>) -> Result<(), Box<dyn StdError>>,
    {
        let registry = Registry::<SqliteAdapter>::try_new(Pool::memory()?, &SCHEMAS)?;

        registry.acquire()?.execute_batch(
            "
            CREATE TABLE users (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL
            );

            CREATE TABLE posts (
                id INTEGER PRIMARY KEY,
                author_id INTEGER,
                title TEXT NOT NULL,
                FOREIGN KEY(author_id) REFERENCES users(id)
            );

            CREATE TABLE profiles (
                id INTEGER PRIMARY KEY,
                user_id INTEGER NOT NULL UNIQUE,
                bio TEXT,
                FOREIGN KEY(user_id) REFERENCES users(id)
            );
            ",
        )?;

        func(&registry)
    }

    fn seed_user(
        registry: &Registry<SqliteAdapter>,
        connection: &Connection,
        id: i64,
        name: &str,
    ) -> Result<(), Error> {
        registry.table("users", connection)?.insert(
            Row::from_iter([
                ("id".to_string(), Attribute::Integer(id)),
                ("name".to_string(), Attribute::Text(name.to_string())),
            ]),
            &QueryParameters::new(&USERS_SCHEMA),
        )?;

        Ok(())
    }

    fn seed_post(
        registry: &Registry<SqliteAdapter>,
        connection: &Connection,
        id: i64,
        author_id: i64,
        title: &str,
    ) -> Result<(), Error> {
        registry.table("posts", connection)?.insert(
            Row::from_iter([
                ("id".to_string(), Attribute::Integer(id)),
                ("author_id".to_string(), Attribute::Integer(author_id)),
                ("title".to_string(), Attribute::Text(title.to_string())),
            ]),
            &QueryParameters::new(&POSTS_SCHEMA),
        )?;

        Ok(())
    }

    fn seed_profile(
        registry: &Registry<SqliteAdapter>,
        connection: &Connection,
        id: i64,
        user_id: i64,
        bio: &str,
    ) -> Result<(), Error> {
        registry.table("profiles", connection)?.insert(
            Row::from_iter([
                ("id".to_string(), Attribute::Integer(id)),
                ("user_id".to_string(), Attribute::Integer(user_id)),
                ("bio".to_string(), Attribute::Text(bio.to_string())),
            ]),
            &QueryParameters::new(&PROFILES_SCHEMA),
        )?;

        Ok(())
    }

    fn new_post(title: &str, author: i64) -> Record<'static> {
        Record::from((
            &POSTS_SCHEMA,
            Attributes::from_iter([("title".to_string(), Attribute::Text(title.to_string()))]),
            Relationships::from_iter([(
                "author",
                Relationship::BelongsTo(Identifier::Integer(author)),
            )]),
        ))
    }

    // --- fetch_record ------------------------------------------------------

    #[test]
    fn test_fetch_record_returns_content() -> Result<(), Box<dyn StdError>> {
        with_registry(|registry| {
            let connection = registry.acquire()?;
            seed_user(registry, &connection, 1, "alice")?;
            seed_post(registry, &connection, 1, 1, "hello")?;

            let store = Store::new(registry, &connection);
            let parameters = QueryParameters::new(&POSTS_SCHEMA);
            let fetched = store.fetch_record(&POSTS_SCHEMA, Identifier::Integer(1), &parameters)?;

            assert_eq!(fetched.content.require_id()?.to_i64()?, 1);
            assert_eq!(fetched.content.require("title")?.as_string()?, "hello");
            assert_eq!(
                fetched.content.require_related("author")?,
                &Relationship::BelongsTo(Identifier::Integer(1))
            );
            assert!(fetched.included.is_empty());

            Ok(())
        })
    }

    #[test]
    fn test_fetch_record_loads_includes() -> Result<(), Box<dyn StdError>> {
        with_registry(|registry| {
            let connection = registry.acquire()?;
            seed_user(registry, &connection, 1, "alice")?;
            seed_post(registry, &connection, 1, 1, "hello")?;

            let store = Store::new(registry, &connection);
            let uri: Uri = "/posts/1?include=author".parse()?;
            let parameters = QueryParameters::parse(&uri, &POSTS_SCHEMA, registry)?;
            let fetched = store.fetch_record(&POSTS_SCHEMA, Identifier::Integer(1), &parameters)?;

            assert_eq!(fetched.included.len(), 1);
            assert_eq!(fetched.included[0].schema.name, "users");
            assert_eq!(fetched.included[0].require_id()?, &Identifier::Integer(1));

            Ok(())
        })
    }

    #[test]
    fn test_fetch_record_missing_is_not_found() -> Result<(), Box<dyn StdError>> {
        with_registry(|registry| {
            let connection = registry.acquire()?;
            let store = Store::new(registry, &connection);

            let parameters = QueryParameters::new(&POSTS_SCHEMA);
            let result = store.fetch_record(&POSTS_SCHEMA, Identifier::Integer(999), &parameters);

            assert!(matches!(result, Err(Error::RecordNotFound)));

            Ok(())
        })
    }

    // --- fetch_collection --------------------------------------------------

    #[test]
    fn test_fetch_collection_returns_all_records() -> Result<(), Box<dyn StdError>> {
        with_registry(|registry| {
            let connection = registry.acquire()?;
            seed_user(registry, &connection, 1, "alice")?;
            seed_post(registry, &connection, 1, 1, "one")?;
            seed_post(registry, &connection, 2, 1, "two")?;
            seed_post(registry, &connection, 3, 1, "three")?;

            let store = Store::new(registry, &connection);
            let parameters = QueryParameters::new(&POSTS_SCHEMA);
            let fetched = store.fetch_collection(&POSTS_SCHEMA, &parameters)?;

            assert_eq!(fetched.content.len(), 3);

            Ok(())
        })
    }

    #[test]
    fn test_fetch_collection_scoped_by_filter() -> Result<(), Box<dyn StdError>> {
        with_registry(|registry| {
            let connection = registry.acquire()?;
            seed_user(registry, &connection, 1, "alice")?;
            seed_user(registry, &connection, 2, "bob")?;
            seed_post(registry, &connection, 1, 1, "alice-one")?;
            seed_post(registry, &connection, 2, 1, "alice-two")?;
            seed_post(registry, &connection, 3, 2, "bob-one")?;

            let store = Store::new(registry, &connection);
            let parameters = QueryParameters {
                filter: Some(FilterParameters::from([(
                    "author_id",
                    vec![FilterValue::Equal(Attribute::Integer(1))],
                )])),
                ..QueryParameters::new(&POSTS_SCHEMA)
            };

            let fetched = store.fetch_collection(&POSTS_SCHEMA, &parameters)?;

            assert_eq!(fetched.content.len(), 2);
            for record in &fetched.content {
                assert_eq!(
                    record.require_related("author")?,
                    &Relationship::BelongsTo(Identifier::Integer(1))
                );
            }

            Ok(())
        })
    }

    #[test]
    fn test_fetch_collection_loads_includes() -> Result<(), Box<dyn StdError>> {
        with_registry(|registry| {
            let connection = registry.acquire()?;
            seed_user(registry, &connection, 1, "alice")?;
            seed_user(registry, &connection, 2, "bob")?;
            seed_post(registry, &connection, 1, 1, "alice-one")?;
            seed_post(registry, &connection, 2, 2, "bob-one")?;

            let store = Store::new(registry, &connection);
            let uri: Uri = "/posts?include=author".parse()?;
            let parameters = QueryParameters::parse(&uri, &POSTS_SCHEMA, registry)?;
            let fetched = store.fetch_collection(&POSTS_SCHEMA, &parameters)?;

            assert_eq!(fetched.content.len(), 2);
            assert_eq!(fetched.included.len(), 2);
            assert!(
                fetched
                    .included
                    .iter()
                    .all(|record| record.schema.name == "users")
            );

            Ok(())
        })
    }

    // --- create_record -----------------------------------------------------

    #[test]
    fn test_create_record_persists_attributes_and_belongs_to() -> Result<(), Box<dyn StdError>> {
        with_registry(|registry| {
            let connection = registry.acquire()?;
            seed_user(registry, &connection, 1, "alice")?;

            let store = Store::new(registry, &connection);
            let parameters = QueryParameters::new(&POSTS_SCHEMA);
            let created = store.create_record(new_post("Hello", 1), &parameters)?;

            assert_eq!(created.content.require("title")?.as_string()?, "Hello");
            assert_eq!(
                created.content.require_related("author")?,
                &Relationship::BelongsTo(Identifier::Integer(1))
            );

            let persisted = registry
                .table("posts", &connection)?
                .query(&QueryParameters::new(&POSTS_SCHEMA))?;
            assert_eq!(persisted.len(), 1);
            assert_eq!(persisted[0]["author_id"], Attribute::Integer(1));

            Ok(())
        })
    }

    #[test]
    fn test_create_record_links_has_many() -> Result<(), Box<dyn StdError>> {
        with_registry(|registry| {
            let connection = registry.acquire()?;
            seed_user(registry, &connection, 1, "alice")?;
            seed_post(registry, &connection, 1, 1, "one")?;
            seed_post(registry, &connection, 2, 1, "two")?;

            let store = Store::new(registry, &connection);
            let attributes =
                Attributes::from_iter([("name".to_string(), Attribute::Text("dave".to_string()))]);
            let relationships = Relationships::from_iter([(
                "posts",
                Relationship::HasMany(vec![Identifier::Integer(1), Identifier::Integer(2)]),
            )]);
            let user = Record::from((&USERS_SCHEMA, attributes, relationships));

            let parameters = QueryParameters::new(&USERS_SCHEMA);
            let created = store.create_record(user, &parameters)?;
            let new_id = *created.content.require_id()?.as_i64()?;

            let posts = registry
                .table("posts", &connection)?
                .query(&QueryParameters::new(&POSTS_SCHEMA))?;
            assert_eq!(posts.len(), 2);
            for post in &posts {
                assert_eq!(post["author_id"], Attribute::Integer(new_id));
            }

            Ok(())
        })
    }

    // --- update_record -----------------------------------------------------

    #[test]
    fn test_update_record_updates_attributes() -> Result<(), Box<dyn StdError>> {
        with_registry(|registry| {
            let connection = registry.acquire()?;
            seed_user(registry, &connection, 1, "alice")?;
            seed_post(registry, &connection, 1, 1, "before")?;

            let store = Store::new(registry, &connection);
            let record = Record::from_attributes(
                &POSTS_SCHEMA,
                Attributes::from_iter([(
                    "title".to_string(),
                    Attribute::Text("after".to_string()),
                )]),
            )
            .with_id(Identifier::Integer(1).into());

            let parameters = QueryParameters::new(&POSTS_SCHEMA);
            store.update_record(record, &parameters)?;

            let posts = registry
                .table("posts", &connection)?
                .query(&QueryParameters::new(&POSTS_SCHEMA))?;
            assert_eq!(posts.len(), 1);
            assert_eq!(posts[0]["title"], Attribute::Text("after".to_string()));

            Ok(())
        })
    }

    #[test]
    fn test_update_record_replaces_has_many() -> Result<(), Box<dyn StdError>> {
        with_registry(|registry| {
            let connection = registry.acquire()?;
            seed_user(registry, &connection, 1, "alice")?;
            seed_user(registry, &connection, 2, "bob")?;
            seed_post(registry, &connection, 1, 1, "p1")?;
            seed_post(registry, &connection, 2, 1, "p2")?;
            seed_post(registry, &connection, 3, 2, "p3")?;

            let store = Store::new(registry, &connection);

            // Reassign bob's posts to exactly {p1}: p1 is adopted, p3 (bob's) is detached.
            let record = Record::from_relationships(
                &USERS_SCHEMA,
                Relationships::from([(
                    "posts",
                    Relationship::HasMany(vec![Identifier::Integer(1)]),
                )]),
            )
            .with_id(Some(Identifier::Integer(2)));
            store.update_record(record, &QueryParameters::new(&USERS_SCHEMA))?;

            let posts: HashMap<Attribute, Row> = registry
                .table("posts", &connection)?
                .query(&QueryParameters::new(&POSTS_SCHEMA))?
                .into_iter()
                .map(|row| (row["id"].clone(), row))
                .collect();

            assert_eq!(
                posts[&Attribute::Integer(1)]["author_id"],
                Attribute::Integer(2)
            );
            assert_eq!(
                posts[&Attribute::Integer(2)]["author_id"],
                Attribute::Integer(1)
            );
            assert_eq!(posts[&Attribute::Integer(3)]["author_id"], Attribute::Null);

            Ok(())
        })
    }

    // --- delete_record -----------------------------------------------------

    #[test]
    fn test_delete_record_removes_row() -> Result<(), Box<dyn StdError>> {
        with_registry(|registry| {
            let connection = registry.acquire()?;
            seed_user(registry, &connection, 1, "alice")?;
            seed_post(registry, &connection, 1, 1, "doomed")?;

            let store = Store::new(registry, &connection);
            store.delete_record(&POSTS_SCHEMA, Identifier::Integer(1))?;

            assert!(
                registry
                    .table("posts", &connection)?
                    .query(&QueryParameters::new(&POSTS_SCHEMA))?
                    .is_empty()
            );

            Ok(())
        })
    }

    // --- create_collection -------------------------------------------------

    #[test]
    fn test_create_collection_inserts_records_with_belongs_to() -> Result<(), Box<dyn StdError>> {
        with_registry(|registry| {
            let connection = registry.acquire()?;
            seed_user(registry, &connection, 1, "alice")?;

            let store = Store::new(registry, &connection);
            let created = store.create_collection(
                vec![new_post("First", 1), new_post("Second", 1)],
                &QueryParameters::new(&POSTS_SCHEMA),
            )?;

            assert_eq!(created.content.len(), 2);
            for record in &created.content {
                assert_eq!(
                    record.require_related("author")?,
                    &Relationship::BelongsTo(Identifier::Integer(1))
                );
            }

            let mut titles = created
                .content
                .iter()
                .map(|record| record.require("title")?.as_string().map(String::as_str))
                .collect::<Result<Vec<_>, Error>>()?;
            titles.sort_unstable();
            assert_eq!(titles, ["First", "Second"]);

            assert_eq!(
                registry
                    .table("posts", &connection)?
                    .query(&QueryParameters::new(&POSTS_SCHEMA))?
                    .len(),
                2
            );

            Ok(())
        })
    }

    #[test]
    fn test_create_collection_assigns_distinct_belongs_to_per_record()
    -> Result<(), Box<dyn StdError>> {
        with_registry(|registry| {
            let connection = registry.acquire()?;
            seed_user(registry, &connection, 1, "alice")?;
            seed_user(registry, &connection, 2, "bob")?;

            let store = Store::new(registry, &connection);
            let posts: HashMap<String, Record> = store
                .create_collection(
                    vec![new_post("alice-post", 1), new_post("bob-post", 2)],
                    &QueryParameters::new(&POSTS_SCHEMA),
                )?
                .content
                .into_iter()
                .map(|post| -> Result<_, Box<dyn StdError>> {
                    Ok((post.require("title")?.as_string()?.clone(), post))
                })
                .try_collect()?;

            assert_eq!(
                posts
                    .get("alice-post")
                    .expect("Alice's post should be in the index")
                    .require_related("author")?,
                &Relationship::BelongsTo(Identifier::Integer(1))
            );
            assert_eq!(
                posts
                    .get("bob-post")
                    .expect("Bob's post should be in the index")
                    .require_related("author")?,
                &Relationship::BelongsTo(Identifier::Integer(2))
            );

            Ok(())
        })
    }

    #[test]
    fn test_create_collection_loads_includes() -> Result<(), Box<dyn StdError>> {
        with_registry(|registry| {
            let connection = registry.acquire()?;
            seed_user(registry, &connection, 1, "alice")?;

            let store = Store::new(registry, &connection);
            let uri: Uri = "/posts?include=author".parse()?;
            let parameters = QueryParameters::parse(&uri, &POSTS_SCHEMA, registry)?;
            let created = store.create_collection(
                vec![new_post("First", 1), new_post("Second", 1)],
                &parameters,
            )?;

            assert_eq!(created.included.len(), 1);
            assert_eq!(created.included[0].schema.name, "users");
            assert_eq!(created.included[0].id, Some(Identifier::Integer(1)));

            Ok(())
        })
    }

    #[test]
    fn test_create_collection_empty_is_a_noop() -> Result<(), Box<dyn StdError>> {
        with_registry(|registry| {
            let connection = registry.acquire()?;
            let store = Store::new(registry, &connection);

            let parameters = QueryParameters::new(&POSTS_SCHEMA);
            let created = store.create_collection(vec![], &parameters)?;

            assert!(created.content.is_empty());
            assert!(created.included.is_empty());
            assert!(
                registry
                    .table("posts", &connection)?
                    .query(&QueryParameters::new(&POSTS_SCHEMA))?
                    .is_empty()
            );

            Ok(())
        })
    }

    #[test]
    fn test_create_collection_links_has_many() -> Result<(), Box<dyn StdError>> {
        with_registry(|registry| {
            let connection = registry.acquire()?;
            seed_user(registry, &connection, 1, "alice")?;
            seed_post(registry, &connection, 1, 1, "one")?;
            seed_post(registry, &connection, 2, 1, "two")?;

            let store = Store::new(registry, &connection);
            let user = Record::from((
                &USERS_SCHEMA,
                Attributes::from_iter([("name".to_string(), Attribute::Text("dave".to_string()))]),
                Relationships::from_iter([(
                    "posts",
                    Relationship::HasMany(vec![Identifier::Integer(1), Identifier::Integer(2)]),
                )]),
            ));
            let created =
                store.create_collection(vec![user], &QueryParameters::new(&USERS_SCHEMA))?;
            let new_id = *created.content[0].require_id()?.as_i64()?;

            let posts = registry
                .table("posts", &connection)?
                .query(&QueryParameters::new(&POSTS_SCHEMA))?;
            assert_eq!(posts.len(), 2);
            for post in &posts {
                assert_eq!(post["author_id"], Attribute::Integer(new_id));
            }

            Ok(())
        })
    }

    #[test]
    fn test_create_collection_links_has_one() -> Result<(), Box<dyn StdError>> {
        with_registry(|registry| {
            let connection = registry.acquire()?;
            seed_user(registry, &connection, 1, "alice")?;
            seed_profile(registry, &connection, 1, 1, "alice's profile")?;

            let store = Store::new(registry, &connection);
            let user = Record::from((
                &USERS_SCHEMA,
                Attributes::from_iter([("name".to_string(), Attribute::Text("dave".to_string()))]),
                Relationships::from_iter([(
                    "profile",
                    Relationship::HasOne(Identifier::Integer(1)),
                )]),
            ));
            let created =
                store.create_collection(vec![user], &QueryParameters::new(&USERS_SCHEMA))?;
            let new_id = *created.content[0].require_id()?.as_i64()?;

            let profile = registry.table("profiles", &connection)?.find(
                Identifier::Integer(1),
                &QueryParameters::new(&PROFILES_SCHEMA),
            )?;
            assert_eq!(profile["user_id"], Attribute::Integer(new_id));

            Ok(())
        })
    }

    // --- update_collection -------------------------------------------------

    #[test]
    fn test_update_collection_uniform_attribute_patch_scoped_by_filter()
    -> Result<(), Box<dyn StdError>> {
        with_registry(|registry| {
            let connection = registry.acquire()?;
            seed_user(registry, &connection, 1, "alice")?;
            seed_user(registry, &connection, 2, "bob")?;
            seed_post(registry, &connection, 1, 1, "alice-one")?;
            seed_post(registry, &connection, 2, 1, "alice-two")?;
            seed_post(registry, &connection, 3, 2, "bob-one")?;

            let store = Store::new(registry, &connection);

            let patch = RecordPatch::from_attributes(
                &POSTS_SCHEMA,
                Attributes::from_iter([(
                    "title".to_string(),
                    Attribute::Text("patched".to_string()),
                )]),
            );

            let parameters = QueryParameters {
                filter: Some(FilterParameters::from([(
                    "author_id",
                    vec![FilterValue::Equal(Attribute::Integer(1))],
                )])),
                ..QueryParameters::new(&POSTS_SCHEMA)
            };

            let updated = store.update_collection(patch, &parameters)?;
            assert_eq!(updated.content.len(), 2);

            let posts = registry
                .table("posts", &connection)?
                .query(&QueryParameters::new(&POSTS_SCHEMA))?;
            for post in &posts {
                let expected = if post["author_id"] == Attribute::Integer(1) {
                    "patched"
                } else {
                    "bob-one"
                };
                assert_eq!(post["title"], Attribute::Text(expected.to_string()));
            }

            Ok(())
        })
    }

    #[test]
    fn test_update_collection_bulk_reassigns_belongs_to() -> Result<(), Box<dyn StdError>> {
        with_registry(|registry| {
            let connection = registry.acquire()?;
            seed_user(registry, &connection, 1, "alice")?;
            seed_user(registry, &connection, 2, "bob")?;
            seed_post(registry, &connection, 1, 1, "one")?;
            seed_post(registry, &connection, 2, 1, "two")?;

            let store = Store::new(registry, &connection);

            let patch = RecordPatch::from_relationships(
                &POSTS_SCHEMA,
                Relationships::from_iter([(
                    "author",
                    Relationship::BelongsTo(Identifier::Integer(2)),
                )]),
            );

            let parameters = QueryParameters::new(&POSTS_SCHEMA);
            store.update_collection(patch, &parameters)?;

            let posts = registry
                .table("posts", &connection)?
                .query(&QueryParameters::new(&POSTS_SCHEMA))?;
            assert_eq!(posts.len(), 2);
            for post in &posts {
                assert_eq!(post["author_id"], Attribute::Integer(2));
            }

            Ok(())
        })
    }

    // --- delete_collection -------------------------------------------------

    #[test]
    fn test_delete_collection_scoped_by_filter() -> Result<(), Box<dyn StdError>> {
        with_registry(|registry| {
            let connection = registry.acquire()?;
            seed_user(registry, &connection, 1, "alice")?;
            seed_user(registry, &connection, 2, "bob")?;
            seed_post(registry, &connection, 1, 1, "alice-one")?;
            seed_post(registry, &connection, 2, 1, "alice-two")?;
            seed_post(registry, &connection, 3, 2, "bob-one")?;

            let store = Store::new(registry, &connection);

            let parameters = QueryParameters {
                filter: Some(FilterParameters::from([(
                    "author_id",
                    vec![FilterValue::Equal(Attribute::Integer(1))],
                )])),
                ..QueryParameters::new(&POSTS_SCHEMA)
            };

            store.delete_collection(&POSTS_SCHEMA, &parameters)?;

            let posts = registry
                .table("posts", &connection)?
                .query(&QueryParameters::new(&POSTS_SCHEMA))?;
            assert_eq!(posts.len(), 1);
            assert_eq!(posts[0]["title"], Attribute::Text("bob-one".to_string()));

            Ok(())
        })
    }

    #[test]
    fn test_delete_collection_unscoped_clears_table() -> Result<(), Box<dyn StdError>> {
        with_registry(|registry| {
            let connection = registry.acquire()?;
            seed_user(registry, &connection, 1, "alice")?;
            seed_post(registry, &connection, 1, 1, "one")?;
            seed_post(registry, &connection, 2, 1, "two")?;

            let store = Store::new(registry, &connection);
            let parameters = QueryParameters::new(&POSTS_SCHEMA);
            store.delete_collection(&POSTS_SCHEMA, &parameters)?;

            assert!(
                registry
                    .table("posts", &connection)?
                    .query(&QueryParameters::new(&POSTS_SCHEMA))?
                    .is_empty()
            );

            Ok(())
        })
    }
}
