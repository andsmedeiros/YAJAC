use crate::database::adapters::Adapter as AdapterInterface;
use crate::database::attributes::{Attribute, Attributes, ForeignKeys, Identifier, from_value};
use crate::database::composite::{Composite, CompositeCollection, CompositeRecord};
use crate::database::connection::Connection as ConnectionInterface;
use crate::database::data_loader::DataLoader;
use crate::database::error::Error;
use crate::database::query_parameters::{FilterParameters, FilterValue, QueryParameters};
use crate::database::record::{NewRecord, Record};
use crate::database::registry::Registry;
use crate::database::relationships::{Relationship as DatabaseRelationship, Relationships};
use crate::database::schema::{
    IdentifierType, RelatedResource, Relationship as SchemaRelationship, TableSchema,
};
use crate::database::table::Table as TableInterface;
use crate::json_api::identifier::Identifier as ResourceIdentifier;
use crate::json_api::relationship::Linkage;
use crate::json_api::resource::Resource;
use indexmap::IndexSet;
use serde_json::Value;

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
            let mut content = self.table(schema)?.find(id, parameters)?;
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
            let mut content = self.table(schema)?.query(parameters)?;
            let included = self
                .loader()
                .load_for_collection(&mut content, parameters)?;

            Ok(Composite { content, included })
        })
    }

    pub fn create_record(
        &self,
        new_record: NewRecord<'sch>,
        parameters: &QueryParameters<'sch, 'req>,
    ) -> Result<CompositeRecord<'sch>, Error> {
        self.connection.transaction(|| {
            let mut content = self
                .table(new_record.schema)?
                .insert(new_record.attributes, parameters)?;
            self.replace_relationships(&content, &new_record.relationships)?;
            let included = self.loader().load_for_record(&mut content, parameters)?;

            Ok(Composite { content, included })
        })
    }

    pub fn update_record(
        &self,
        record: Record<'sch>,
        parameters: &QueryParameters<'sch, 'req>,
    ) -> Result<CompositeRecord<'sch>, Error> {
        self.connection.transaction(|| {
            let table = self.table(record.schema)?;
            let mut content = if record.attributes.is_empty() {
                table.find(record.id.clone(), parameters)?
            } else {
                table.update(record.id.clone(), record.attributes, parameters)?
            };
            self.replace_relationships(&content, &record.relationships)?;
            let included = self.loader().load_for_record(&mut content, parameters)?;

            Ok(Composite { content, included })
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
        _records: Vec<NewRecord<'sch>>,
        _parameters: &QueryParameters<'sch, 'req>,
    ) -> Result<CompositeCollection<'sch>, Error> {
        unimplemented!()
    }

    pub fn update_collection(
        &self,
        _patch: NewRecord<'sch>,
        _parameters: &QueryParameters<'sch, 'req>,
    ) -> Result<CompositeCollection<'sch>, Error> {
        unimplemented!()
    }

    pub fn delete_collection(
        &self,
        _schema: &'sch TableSchema<'sch>,
        _parameters: &QueryParameters<'sch, 'req>,
    ) -> Result<(), Error> {
        unimplemented!()
    }

    pub fn materialise_new(
        &self,
        resource: &Resource,
        schema: &'sch TableSchema<'sch>,
    ) -> Result<NewRecord<'sch>, Error> {
        let (attributes, relationships) = self.hydrate(resource, schema)?;

        Ok(NewRecord {
            schema,
            attributes,
            relationships,
        })
    }

    pub fn materialise(
        &self,
        resource: &Resource,
        schema: &'sch TableSchema<'sch>,
        id: Identifier,
    ) -> Result<Record<'sch>, Error> {
        let (attributes, relationships) = self.hydrate(resource, schema)?;
        let mut record = Record::new(schema, id, attributes, ForeignKeys::new());
        record.relationships = relationships;

        Ok(record)
    }

    fn hydrate(
        &self,
        resource: &Resource,
        schema: &'sch TableSchema<'sch>,
    ) -> Result<(Attributes, Relationships<'sch>), Error> {
        let mut attributes = match &resource.attributes {
            Some(map) => from_value(schema, Value::Object(map.clone().into_iter().collect()))?,
            None => Attributes::new(),
        };
        let mut relationships = Relationships::new();

        for (name, relationship) in resource.relationships.iter().flatten() {
            let (key, descriptor) = self.relationship(schema, name)?;
            let related = descriptor.related_resource();
            let linkage = relationship.data.clone().unwrap_or(Linkage::Empty);

            match descriptor {
                SchemaRelationship::BelongsTo(_) => {
                    let value = match linkage {
                        Linkage::Empty => Attribute::Null,
                        Linkage::ToOne(identifier) => {
                            self.belongs_to_value(related, &identifier)?
                        }
                        Linkage::ToMany(_) => return Err(Self::mismatch(schema, name)),
                    };
                    attributes.insert(related.keys.own.to_string(), value);
                }
                SchemaRelationship::HasOne(_) | SchemaRelationship::HasMany(_) => {
                    relationships.insert(
                        key,
                        DatabaseRelationship::HasMany(self.members(related, linkage)?),
                    );
                }
            }
        }

        Ok((attributes, relationships))
    }

    fn replace_relationships(
        &self,
        owner: &Record<'sch>,
        relationships: &Relationships<'sch>,
    ) -> Result<(), Error> {
        for (name, relationship) in relationships {
            let DatabaseRelationship::HasMany(members) = relationship else {
                continue;
            };

            let related = self.relationship(owner.schema, name)?.1.related_resource();
            self.replace_relationship(owner, related, members)?;
        }

        Ok(())
    }

    fn replace_relationship(
        &self,
        owner: &Record<'sch>,
        related: &RelatedResource<'sch>,
        members: &[Identifier],
    ) -> Result<(), Error> {
        let owner_value = Self::value_at(owner, related.keys.own)?;
        let related_schema = self.registry.schema(related.resource)?;
        let table = self.table(related_schema)?;
        let foreign_key = related.keys.related;
        let primary_key = related_schema.primary_key.name;

        let mut detach =
            FilterParameters::from([(foreign_key, vec![FilterValue::Equal(owner_value.clone())])]);
        if !members.is_empty() {
            detach.insert(primary_key, vec![FilterValue::NotIn(member_set(members))]);
        }
        table.update_batch(
            Attributes::from_iter([(foreign_key.to_string(), Attribute::Null)]),
            &scope(related_schema, detach),
        )?;

        if !members.is_empty() {
            let adopt =
                FilterParameters::from([(primary_key, vec![FilterValue::In(member_set(members))])]);
            table.update_batch(
                Attributes::from_iter([(foreign_key.to_string(), owner_value)]),
                &scope(related_schema, adopt),
            )?;
        }

        Ok(())
    }

    fn members(
        &self,
        related: &RelatedResource<'sch>,
        linkage: Linkage,
    ) -> Result<Vec<Identifier>, Error> {
        let kind = self.registry.schema(related.resource)?.primary_key.kind;

        match linkage {
            Linkage::Empty => Ok(Vec::new()),
            Linkage::ToOne(identifier) => Ok(vec![target_identifier(&identifier, kind)?]),
            Linkage::ToMany(identifiers) => identifiers
                .iter()
                .map(|identifier| target_identifier(identifier, kind))
                .collect(),
        }
    }

    fn belongs_to_value(
        &self,
        related: &RelatedResource<'sch>,
        linkage: &ResourceIdentifier,
    ) -> Result<Attribute, Error> {
        let related_schema = self.registry.schema(related.resource)?;
        let target = target_identifier(linkage, related_schema.primary_key.kind)?;

        if related_schema.is_primary_key(related.keys.related) {
            Ok(Attribute::from(target))
        } else {
            let record = self
                .table(related_schema)?
                .find(target, &QueryParameters::new(related_schema))?;
            Self::value_at(&record, related.keys.related)
        }
    }

    fn relationship(
        &self,
        schema: &'sch TableSchema<'sch>,
        name: &str,
    ) -> Result<(&'sch str, &'sch SchemaRelationship<'sch>), Error> {
        schema
            .relationships
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(key, descriptor)| (*key, descriptor))
            .ok_or_else(|| Error::SchemaValidationFailure {
                schema: schema.name.to_string(),
                attribute: name.to_string(),
                message: "Unknown relationship".to_string(),
            })
    }

    fn value_at(record: &Record, column: &str) -> Result<Attribute, Error> {
        if record.schema.is_primary_key(column) {
            Ok(Attribute::from(record.id.clone()))
        } else {
            record
                .attributes
                .get(column)
                .or_else(|| record.foreign_keys.get(column))
                .map(ToOwned::to_owned)
                .ok_or_else(|| Error::DataLoadingError {
                    message: format!(
                        "Column '{column}' was not loaded on '{}'",
                        record.schema.name
                    ),
                })
        }
    }

    fn mismatch(schema: &TableSchema, name: &str) -> Error {
        Error::InconsistentSchema {
            schema: schema.name.to_string(),
            attribute: name.to_string(),
            message: "Relationship kind does not match schema".to_string(),
        }
    }

    fn table(&self, schema: &'sch TableSchema<'sch>) -> Result<Adapter::Table<'sch, 'req>, Error> {
        self.registry.table(schema.name, self.connection)
    }

    fn loader(&self) -> DataLoader<'sch, 'req, Adapter> {
        DataLoader::new(self.registry, self.connection)
    }
}

fn target_identifier(
    identifier: &ResourceIdentifier,
    kind: IdentifierType,
) -> Result<Identifier, Error> {
    let ResourceIdentifier::Existing { id, .. } = identifier else {
        return Err(Error::InvalidAttributeSet);
    };

    match kind {
        IdentifierType::Integer => {
            id.parse()
                .map(Identifier::Integer)
                .map_err(|_| Error::InvalidAttribute {
                    attribute: id.clone(),
                    kind: "Integer".to_string(),
                    message: "linkage id is not a valid integer".to_string(),
                })
        }
        IdentifierType::Text => Ok(Identifier::Text(id.clone())),
    }
}

fn member_set(members: &[Identifier]) -> IndexSet<Attribute> {
    members.iter().cloned().map(Attribute::from).collect()
}

fn scope<'sch>(
    schema: &'sch TableSchema<'sch>,
    filter: FilterParameters<'sch>,
) -> QueryParameters<'sch, 'sch> {
    let mut parameters = QueryParameters::new(schema);
    parameters.filter = Some(filter);
    parameters
}

#[cfg(test)]
mod tests {
    use super::Store;
    use crate::database::adapters::SqliteAdapter;
    use crate::database::adapters::sqlite::{Connection, Pool};
    use crate::database::attributes::{Attribute, Attributes, ForeignKeys, Identifier};
    use crate::database::error::Error;
    use crate::database::query_parameters::{FilterParameters, FilterValue, QueryParameters};
    use crate::database::record::{NewRecord, Record};
    use crate::database::registry::Registry;
    use crate::database::relationships::Relationship;
    use crate::database::schema::{
        AttributeType, IdentifierType, PrimaryKey, RelatedResource,
        Relationship as SchemaRelationship, RelationshipKeys, TableSchema,
    };
    use crate::database::table::Table;
    use crate::http_wrappers::Uri;
    use std::error::Error as StdError;

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
            Attributes::from_iter([
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
            Attributes::from_iter([
                ("id".to_string(), Attribute::Integer(id)),
                ("author_id".to_string(), Attribute::Integer(author_id)),
                ("title".to_string(), Attribute::Text(title.to_string())),
            ]),
            &QueryParameters::new(&POSTS_SCHEMA),
        )?;

        Ok(())
    }

    fn all_posts<'a>(
        registry: &Registry<'a, SqliteAdapter>,
        connection: &Connection,
    ) -> Result<Vec<Record<'a>>, Error> {
        registry
            .table("posts", connection)?
            .query(&QueryParameters::new(&POSTS_SCHEMA))
    }

    fn title_of<'a>(record: &'a Record) -> &'a str {
        record.attributes["title"]
            .as_string()
            .expect("title should be a text attribute")
    }

    fn author_of<'a>(record: &'a Record) -> Option<&'a Attribute> {
        record.foreign_keys.get("author_id")
    }

    fn new_post(title: &str, author: i64) -> NewRecord<'static> {
        let mut post = NewRecord::new(&POSTS_SCHEMA);
        post.attributes
            .insert("title".to_string(), Attribute::Text(title.to_string()));
        post.relationships.insert(
            "author",
            Relationship::BelongsTo(Identifier::Integer(author)),
        );
        post
    }

    // --- fetch_record ------------------------------------------------------

    #[test]
    fn test_fetch_record_returns_content() -> Result<(), Box<dyn StdError>> {
        with_registry(|registry| {
            let connection = registry.acquire()?;
            seed_user(registry, &connection, 1, "alice")?;
            seed_post(registry, &connection, 1, 1, "hello")?;

            let store = Store::new(registry, &connection);
            let uri: Uri = "/".parse()?;
            let parameters = QueryParameters::parse(&uri, &POSTS_SCHEMA, registry)?;
            let fetched = store.fetch_record(&POSTS_SCHEMA, Identifier::Integer(1), &parameters)?;

            assert_eq!(fetched.content.id, Identifier::Integer(1));
            assert_eq!(title_of(&fetched.content), "hello");
            assert_eq!(author_of(&fetched.content), Some(&Attribute::Integer(1)));
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
            assert_eq!(fetched.included[0].id, Identifier::Integer(1));

            Ok(())
        })
    }

    #[test]
    fn test_fetch_record_missing_is_not_found() -> Result<(), Box<dyn StdError>> {
        with_registry(|registry| {
            let connection = registry.acquire()?;
            let store = Store::new(registry, &connection);

            let uri: Uri = "/".parse()?;
            let parameters = QueryParameters::parse(&uri, &POSTS_SCHEMA, registry)?;
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
            let uri: Uri = "/".parse()?;
            let parameters = QueryParameters::parse(&uri, &POSTS_SCHEMA, registry)?;
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
            let uri: Uri = "/".parse()?;
            let mut parameters = QueryParameters::parse(&uri, &POSTS_SCHEMA, registry)?;
            parameters.filter = Some(FilterParameters::from([(
                "author_id",
                vec![FilterValue::Equal(Attribute::Integer(1))],
            )]));

            let fetched = store.fetch_collection(&POSTS_SCHEMA, &parameters)?;

            assert_eq!(fetched.content.len(), 2);
            for record in &fetched.content {
                assert_eq!(author_of(record), Some(&Attribute::Integer(1)));
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
            let uri: Uri = "/".parse()?;
            let parameters = QueryParameters::parse(&uri, &POSTS_SCHEMA, registry)?;
            let created = store.create_record(new_post("Hello", 1), &parameters)?;

            assert_eq!(title_of(&created.content), "Hello");
            assert_eq!(author_of(&created.content), Some(&Attribute::Integer(1)));

            let persisted = all_posts(registry, &connection)?;
            assert_eq!(persisted.len(), 1);
            assert_eq!(author_of(&persisted[0]), Some(&Attribute::Integer(1)));

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

            let mut user = NewRecord::new(&USERS_SCHEMA);
            user.attributes
                .insert("name".to_string(), Attribute::Text("dave".to_string()));
            user.relationships.insert(
                "posts",
                Relationship::HasMany(vec![Identifier::Integer(1), Identifier::Integer(2)]),
            );

            let uri: Uri = "/".parse()?;
            let parameters = QueryParameters::parse(&uri, &USERS_SCHEMA, registry)?;
            let created = store.create_record(user, &parameters)?;
            let new_id = *created.content.id.as_i64()?;

            let posts = all_posts(registry, &connection)?;
            assert_eq!(posts.len(), 2);
            for post in &posts {
                assert_eq!(author_of(post), Some(&Attribute::Integer(new_id)));
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
            let record = Record::new(
                &POSTS_SCHEMA,
                Identifier::Integer(1),
                Attributes::from_iter([(
                    "title".to_string(),
                    Attribute::Text("after".to_string()),
                )]),
                ForeignKeys::new(),
            );

            let uri: Uri = "/".parse()?;
            let parameters = QueryParameters::parse(&uri, &POSTS_SCHEMA, registry)?;
            store.update_record(record, &parameters)?;

            let posts = all_posts(registry, &connection)?;
            assert_eq!(posts.len(), 1);
            assert_eq!(title_of(&posts[0]), "after");

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
            let mut record = Record::new(
                &USERS_SCHEMA,
                Identifier::Integer(2),
                Attributes::new(),
                ForeignKeys::new(),
            );
            record
                .relationships
                .insert("posts", Relationship::HasMany(vec![Identifier::Integer(1)]));

            let uri: Uri = "/".parse()?;
            let parameters = QueryParameters::parse(&uri, &USERS_SCHEMA, registry)?;
            store.update_record(record, &parameters)?;

            let posts = all_posts(registry, &connection)?;
            let author = |id: i64| {
                posts
                    .iter()
                    .find(|post| post.id == Identifier::Integer(id))
                    .map(author_of)
                    .expect("post should exist")
            };

            assert_eq!(author(1), Some(&Attribute::Integer(2)));
            assert_eq!(author(2), Some(&Attribute::Integer(1)));
            assert_eq!(author(3), Some(&Attribute::Null));

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

            assert!(all_posts(registry, &connection)?.is_empty());

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
            let uri: Uri = "/".parse()?;
            let parameters = QueryParameters::parse(&uri, &POSTS_SCHEMA, registry)?;
            let created = store.create_collection(
                vec![new_post("First", 1), new_post("Second", 1)],
                &parameters,
            )?;

            assert_eq!(created.content.len(), 2);
            for record in &created.content {
                assert_eq!(author_of(record), Some(&Attribute::Integer(1)));
            }

            let mut titles: Vec<&str> = created.content.iter().map(title_of).collect();
            titles.sort_unstable();
            assert_eq!(titles, ["First", "Second"]);

            assert_eq!(all_posts(registry, &connection)?.len(), 2);

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
            let uri: Uri = "/".parse()?;
            let parameters = QueryParameters::parse(&uri, &POSTS_SCHEMA, registry)?;
            store.create_collection(
                vec![new_post("alice-post", 1), new_post("bob-post", 2)],
                &parameters,
            )?;

            let posts = all_posts(registry, &connection)?;
            let author = |title: &str| {
                posts
                    .iter()
                    .find(|post| title_of(post) == title)
                    .map(author_of)
                    .expect("post should exist")
            };

            assert_eq!(author("alice-post"), Some(&Attribute::Integer(1)));
            assert_eq!(author("bob-post"), Some(&Attribute::Integer(2)));

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
            assert_eq!(created.included[0].id, Identifier::Integer(1));

            Ok(())
        })
    }

    #[test]
    fn test_create_collection_empty_is_a_noop() -> Result<(), Box<dyn StdError>> {
        with_registry(|registry| {
            let connection = registry.acquire()?;
            let store = Store::new(registry, &connection);

            let uri: Uri = "/".parse()?;
            let parameters = QueryParameters::parse(&uri, &POSTS_SCHEMA, registry)?;
            let created = store.create_collection(vec![], &parameters)?;

            assert!(created.content.is_empty());
            assert!(created.included.is_empty());
            assert!(all_posts(registry, &connection)?.is_empty());

            Ok(())
        })
    }

    #[test]
    fn test_create_collection_rejects_has_many() -> Result<(), Box<dyn StdError>> {
        with_registry(|registry| {
            let connection = registry.acquire()?;
            let store = Store::new(registry, &connection);

            let mut user = NewRecord::new(&USERS_SCHEMA);
            user.attributes
                .insert("name".to_string(), Attribute::Text("dave".to_string()));
            user.relationships
                .insert("posts", Relationship::HasMany(vec![Identifier::Integer(1)]));

            let uri: Uri = "/".parse()?;
            let parameters = QueryParameters::parse(&uri, &USERS_SCHEMA, registry)?;
            let result = store.create_collection(vec![user], &parameters);

            assert!(matches!(result, Err(Error::InvalidOperation { .. })));

            Ok(())
        })
    }

    #[test]
    fn test_create_collection_rejects_has_one() -> Result<(), Box<dyn StdError>> {
        with_registry(|registry| {
            let connection = registry.acquire()?;
            let store = Store::new(registry, &connection);

            let mut user = NewRecord::new(&USERS_SCHEMA);
            user.attributes
                .insert("name".to_string(), Attribute::Text("dave".to_string()));
            user.relationships
                .insert("profile", Relationship::HasOne(Identifier::Integer(1)));

            let uri: Uri = "/".parse()?;
            let parameters = QueryParameters::parse(&uri, &USERS_SCHEMA, registry)?;
            let result = store.create_collection(vec![user], &parameters);

            assert!(matches!(result, Err(Error::InvalidOperation { .. })));

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

            let mut patch = NewRecord::new(&POSTS_SCHEMA);
            patch
                .attributes
                .insert("title".to_string(), Attribute::Text("patched".to_string()));

            let uri: Uri = "/".parse()?;
            let mut parameters = QueryParameters::parse(&uri, &POSTS_SCHEMA, registry)?;
            parameters.filter = Some(FilterParameters::from([(
                "author_id",
                vec![FilterValue::Equal(Attribute::Integer(1))],
            )]));

            let updated = store.update_collection(patch, &parameters)?;
            assert_eq!(updated.content.len(), 2);

            let posts = all_posts(registry, &connection)?;
            for post in &posts {
                let expected = if author_of(post) == Some(&Attribute::Integer(1)) {
                    "patched"
                } else {
                    "bob-one"
                };
                assert_eq!(title_of(post), expected);
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

            let mut patch = NewRecord::new(&POSTS_SCHEMA);
            patch
                .relationships
                .insert("author", Relationship::BelongsTo(Identifier::Integer(2)));

            let uri: Uri = "/".parse()?;
            let parameters = QueryParameters::parse(&uri, &POSTS_SCHEMA, registry)?;
            store.update_collection(patch, &parameters)?;

            let posts = all_posts(registry, &connection)?;
            assert_eq!(posts.len(), 2);
            for post in &posts {
                assert_eq!(author_of(post), Some(&Attribute::Integer(2)));
            }

            Ok(())
        })
    }

    #[test]
    fn test_update_collection_rejects_has_many() -> Result<(), Box<dyn StdError>> {
        with_registry(|registry| {
            let connection = registry.acquire()?;
            let store = Store::new(registry, &connection);

            let mut patch = NewRecord::new(&USERS_SCHEMA);
            patch
                .relationships
                .insert("posts", Relationship::HasMany(vec![Identifier::Integer(1)]));

            let uri: Uri = "/".parse()?;
            let parameters = QueryParameters::parse(&uri, &USERS_SCHEMA, registry)?;
            let result = store.update_collection(patch, &parameters);

            assert!(matches!(result, Err(Error::InvalidOperation { .. })));

            Ok(())
        })
    }

    #[test]
    fn test_update_collection_rejects_has_one() -> Result<(), Box<dyn StdError>> {
        with_registry(|registry| {
            let connection = registry.acquire()?;
            let store = Store::new(registry, &connection);

            let mut patch = NewRecord::new(&USERS_SCHEMA);
            patch
                .relationships
                .insert("profile", Relationship::HasOne(Identifier::Integer(1)));

            let uri: Uri = "/".parse()?;
            let parameters = QueryParameters::parse(&uri, &USERS_SCHEMA, registry)?;
            let result = store.update_collection(patch, &parameters);

            assert!(matches!(result, Err(Error::InvalidOperation { .. })));

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

            let uri: Uri = "/".parse()?;
            let mut parameters = QueryParameters::parse(&uri, &POSTS_SCHEMA, registry)?;
            parameters.filter = Some(FilterParameters::from([(
                "author_id",
                vec![FilterValue::Equal(Attribute::Integer(1))],
            )]));

            store.delete_collection(&POSTS_SCHEMA, &parameters)?;

            let posts = all_posts(registry, &connection)?;
            assert_eq!(posts.len(), 1);
            assert_eq!(title_of(&posts[0]), "bob-one");

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
            let uri: Uri = "/".parse()?;
            let parameters = QueryParameters::parse(&uri, &POSTS_SCHEMA, registry)?;
            store.delete_collection(&POSTS_SCHEMA, &parameters)?;

            assert!(all_posts(registry, &connection)?.is_empty());

            Ok(())
        })
    }
}
