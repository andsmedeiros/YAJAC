use rusqlite::Connection;
use std::error::Error;
use yajac::database::QueryParameters;
use yajac::database::adapters::SqliteAdapter;
use yajac::database::attributes::{Attribute, Attributes};
use yajac::database::data_loader::DataLoader;
use yajac::database::registry::Registry;
use yajac::database::schema::{
    AttributeType, IdentifierType, PrimaryKey, RelatedResource, Relationship, RelationshipKeys,
    TableSchema,
};
use yajac::database::table::Table;
use yajac::{core::to_document, http_wrappers::Uri, routing::DefaultUriGenerator};

static USERS_SCHEMA: TableSchema = TableSchema {
    name: "users",
    primary_key: PrimaryKey {
        name: "id",
        kind: IdentifierType::Integer,
    },
    attributes: &[
        ("username", AttributeType::Text),
        ("email", AttributeType::Text),
    ],
    foreign_keys: &[],
    relationships: &[
        (
            "profile",
            Relationship::HasOne(RelatedResource {
                resource: "profiles",
                keys: RelationshipKeys {
                    related: "user_id",
                    own: "id",
                },
            }),
        ),
        (
            "posts",
            Relationship::HasMany(RelatedResource {
                resource: "posts",
                keys: RelationshipKeys {
                    related: "author_id",
                    own: "id",
                },
            }),
        ),
        (
            "comments",
            Relationship::HasMany(RelatedResource {
                resource: "comments",
                keys: RelationshipKeys {
                    related: "author_id",
                    own: "id",
                },
            }),
        ),
    ],
    text_index: false,
};

static PROFILES_SCHEMA: TableSchema = TableSchema {
    name: "profiles",
    primary_key: PrimaryKey {
        name: "id",
        kind: IdentifierType::Integer,
    },
    attributes: &[
        ("bio", AttributeType::Text),
        ("avatar_url", AttributeType::Text),
    ],
    foreign_keys: &[("user_id", AttributeType::Integer)],
    relationships: &[(
        "user",
        Relationship::BelongsTo(RelatedResource {
            resource: "users",
            keys: RelationshipKeys {
                related: "id",
                own: "user_id",
            },
        }),
    )],
    text_index: false,
};

static POSTS_SCHEMA: TableSchema = TableSchema {
    name: "posts",
    primary_key: PrimaryKey {
        name: "id",
        kind: IdentifierType::Integer,
    },
    attributes: &[
        ("title", AttributeType::Text),
        ("content", AttributeType::Text),
        ("published", AttributeType::Boolean),
    ],
    foreign_keys: &[("author_id", AttributeType::Integer)],
    relationships: &[
        (
            "author",
            Relationship::BelongsTo(RelatedResource {
                resource: "users",
                keys: RelationshipKeys {
                    related: "id",
                    own: "author_id",
                },
            }),
        ),
        (
            "comments",
            Relationship::HasMany(RelatedResource {
                resource: "comments",
                keys: RelationshipKeys {
                    related: "post_id",
                    own: "id",
                },
            }),
        ),
    ],
    text_index: false,
};

static COMMENTS_SCHEMA: TableSchema = TableSchema {
    name: "comments",
    primary_key: PrimaryKey {
        name: "id",
        kind: IdentifierType::Integer,
    },
    attributes: &[("content", AttributeType::Text)],
    foreign_keys: &[
        ("post_id", AttributeType::Integer),
        ("author_id", AttributeType::Integer),
        ("parent_id", AttributeType::Integer),
    ],
    relationships: &[
        (
            "post",
            Relationship::BelongsTo(RelatedResource {
                resource: "posts",
                keys: RelationshipKeys {
                    related: "id",
                    own: "post_id",
                },
            }),
        ),
        (
            "author",
            Relationship::BelongsTo(RelatedResource {
                resource: "users",
                keys: RelationshipKeys {
                    related: "id",
                    own: "author_id",
                },
            }),
        ),
        (
            "parent",
            Relationship::BelongsTo(RelatedResource {
                resource: "comments",
                keys: RelationshipKeys {
                    related: "id",
                    own: "parent_id",
                },
            }),
        ),
        (
            "replies",
            Relationship::HasMany(RelatedResource {
                resource: "comments",
                keys: RelationshipKeys {
                    related: "parent_id",
                    own: "id",
                },
            }),
        ),
    ],
    text_index: false,
};

static TAGS_SCHEMA: TableSchema = TableSchema {
    name: "tags",
    primary_key: PrimaryKey {
        name: "id",
        kind: IdentifierType::Integer,
    },
    attributes: &[("name", AttributeType::Text)],
    foreign_keys: &[],
    relationships: &[],
    text_index: false,
};

static SCHEMAS: [&TableSchema; 5] = [
    &USERS_SCHEMA,
    &PROFILES_SCHEMA,
    &POSTS_SCHEMA,
    &COMMENTS_SCHEMA,
    &TAGS_SCHEMA,
];

fn with_database<F>(func: F) -> Result<(), Box<dyn Error>>
where
    F: FnOnce(&Registry<SqliteAdapter>) -> Result<(), Box<dyn Error>>,
{
    let connection = Connection::open(":memory:")?;

    connection.execute_batch(
        "
        CREATE TABLE users (
            id INTEGER PRIMARY KEY,
            username TEXT NOT NULL,
            email TEXT NOT NULL
        );

        CREATE TABLE profiles (
            id INTEGER PRIMARY KEY,
            user_id INTEGER NOT NULL UNIQUE,
            bio TEXT,
            avatar_url TEXT,
            FOREIGN KEY(user_id) REFERENCES users(id)
        );

        CREATE TABLE posts (
            id INTEGER PRIMARY KEY,
            author_id INTEGER NOT NULL,
            title TEXT NOT NULL,
            content TEXT,
            published BOOLEAN DEFAULT 0,
            FOREIGN KEY(author_id) REFERENCES users(id)
        );

        CREATE TABLE comments (
            id INTEGER PRIMARY KEY,
            post_id INTEGER NOT NULL,
            author_id INTEGER NOT NULL,
            parent_id INTEGER,
            content TEXT NOT NULL,
            FOREIGN KEY(post_id) REFERENCES posts(id),
            FOREIGN KEY(author_id) REFERENCES users(id),
            FOREIGN KEY(parent_id) REFERENCES comments(id)
        );

        CREATE TABLE tags (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL UNIQUE
        );
        ",
    )?;

    let registry = Registry::<SqliteAdapter>::try_new(connection, &SCHEMAS)?;
    func(&registry)?;

    Ok(())
}

fn seed_database(registry: &Registry<SqliteAdapter>) -> Result<(), Box<dyn Error>> {
    use Attribute::{Integer, Null};

    // Create users
    let users_table = registry.table("users")?;
    for (i, (username, email)) in [
        ("alice", "alice@example.com"),
        ("bob", "bob@example.com"),
        ("charlie", "charlie@example.com"),
    ]
    .iter()
    .enumerate()
    {
        users_table.insert(
            Attributes::from_iter([
                ("id".to_string(), Attribute::Integer((i + 1) as i64)),
                (
                    "username".to_string(),
                    Attribute::Text(username.to_string()),
                ),
                ("email".to_string(), Attribute::Text(email.to_string())),
            ]),
            &QueryParameters::new(users_table.schema()),
        )?;
    }

    // Create profiles
    let profiles_table = registry.table("profiles")?;
    for (id, user_id, bio, avatar) in [
        (1, 1, "Alice's bio", "https://example.com/alice.jpg"),
        (2, 2, "Bob's bio", "https://example.com/bob.jpg"),
        (3, 3, "Charlie's bio", "https://example.com/charlie.jpg"),
    ] {
        profiles_table.insert(
            Attributes::from_iter([
                ("id".to_string(), Attribute::Integer(id)),
                ("user_id".to_string(), Attribute::Integer(user_id)),
                ("bio".to_string(), Attribute::Text(bio.to_string())),
                (
                    "avatar_url".to_string(),
                    Attribute::Text(avatar.to_string()),
                ),
            ]),
            &QueryParameters::new(profiles_table.schema()),
        )?;
    }

    // Create posts
    let posts_table = registry.table("posts")?;
    for (id, author_id, title, content, published) in [
        (
            1,
            1,
            "Alice's First Post",
            "Content of Alice's first post",
            true,
        ),
        (
            2,
            1,
            "Alice's Second Post",
            "Content of Alice's second post",
            true,
        ),
        (3, 2, "Bob's Post", "Content of Bob's post", true),
        (4, 2, "Bob's Draft", "This is not published", false),
        (5, 3, "Charlie's Post", "Content of Charlie's post", true),
    ] {
        posts_table.insert(
            Attributes::from_iter([
                ("id".to_string(), Attribute::Integer(id)),
                ("author_id".to_string(), Attribute::Integer(author_id)),
                ("title".to_string(), Attribute::Text(title.to_string())),
                ("content".to_string(), Attribute::Text(content.to_string())),
                ("published".to_string(), Attribute::Boolean(published)),
            ]),
            &QueryParameters::new(posts_table.schema()),
        )?;
    }

    // Create comments (including nested replies for 4-level depth)
    let comments_table = registry.table("comments")?;
    for (id, post_id, author_id, parent_id, content) in [
        // Post 1 comments - 4 levels deep
        (1, 1, 2, Null, "Bob commenting on Alice's first post"),
        (
            2,
            1,
            3,
            Null,
            "Charlie also commenting on Alice's first post",
        ),
        (
            3,
            1,
            1,
            Integer(1),
            "Alice replying to Bob's comment (level 2)",
        ),
        (
            4,
            1,
            3,
            Integer(1),
            "Charlie also replying to Bob's comment (level 2)",
        ),
        (
            5,
            1,
            2,
            Integer(3),
            "Bob replying to Alice's reply (level 3)",
        ),
        (6, 1, 1, Integer(5), "Alice replying again (level 4)"),
        // Post 2 comments
        (7, 2, 2, Null, "Bob commenting on Alice's second post"),
        (8, 2, 3, Null, "Charlie commenting on Alice's second post"),
        // Post 3 comments
        (9, 3, 1, Null, "Alice commenting on Bob's post"),
        (10, 3, 3, Null, "Charlie commenting on Bob's post"),
        (11, 3, 2, Integer(9), "Bob replying to Alice"),
    ] {
        comments_table.insert(
            Attributes::from_iter([
                ("id".to_string(), Attribute::Integer(id)),
                ("post_id".to_string(), Attribute::Integer(post_id)),
                ("author_id".to_string(), Attribute::Integer(author_id)),
                ("parent_id".to_string(), parent_id),
                ("content".to_string(), Attribute::Text(content.to_string())),
            ]),
            &QueryParameters::new(comments_table.schema()),
        )?;
    }

    // Create tags
    let tags_table = registry.table("tags")?;
    for (id, name) in [(1, "rust"), (2, "programming"), (3, "web"), (4, "database")] {
        tags_table.insert(
            Attributes::from_iter([
                ("id".to_string(), Attribute::Integer(id)),
                ("name".to_string(), Attribute::Text(name.to_string())),
            ]),
            &QueryParameters::new(tags_table.schema()),
        )?;
    }

    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    colog::init();

    with_database(|registry| {
        seed_database(&registry)?;

        let uri: Uri = "/users?include=posts.comments.replies.replies".parse()?;
        let schema = registry.table("users")?.schema();
        let query_params = QueryParameters::parse(&uri, schema, registry)?;

        let mut collection = registry.table("users")?.query(&query_params)?;
        let included =
            DataLoader::new(&registry).load_for_collection(&mut collection, &query_params)?;
        let document = to_document(
            &collection,
            included,
            &uri,
            &DefaultUriGenerator::default(),
        )?;
        println!("{}", serde_json::to_string_pretty(&document)?);

        Ok(())
    })
}
