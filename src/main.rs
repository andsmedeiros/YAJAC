use std::error::Error;
use yajac::database::QueryParameters;
use yajac::database::adapters::SqliteAdapter;
use yajac::database::adapters::sqlite::Pool as SqlitePool;
use yajac::database::attributes::{Attribute, Row};
use yajac::database::connection_manager::ConnectionManager;
use yajac::database::data_loader::DataLoader;
use yajac::database::record::Record;
use yajac::database::registry::Registry;
use yajac::database::schema::{AttributeType, Related, SchemaBuilder};
use yajac::database::table::Table;
use yajac::{core::to_document, http_wrappers::Uri, routing::DefaultUriGenerator};

fn users_schema() -> SchemaBuilder<'static> {
    SchemaBuilder::table("users")
        .attribute("username", AttributeType::Text)
        .attribute("email", AttributeType::Text)
        .has_one(
            "profile",
            Related::to("profiles")
                .pointing_related("user_id")
                .to_own("id"),
        )
        .has_many(
            "posts",
            Related::to("posts")
                .pointing_related("author_id")
                .to_own("id"),
        )
        .has_many(
            "comments",
            Related::to("comments")
                .pointing_related("author_id")
                .to_own("id"),
        )
}

fn profiles_schema() -> SchemaBuilder<'static> {
    SchemaBuilder::table("profiles")
        .attribute("bio", AttributeType::Text)
        .attribute("avatar_url", AttributeType::Text)
        .foreign_key("user_id", AttributeType::Integer)
        .belongs_to(
            "user",
            Related::to("users")
                .pointing_own("user_id")
                .to_related("id"),
        )
}

fn posts_schema() -> SchemaBuilder<'static> {
    SchemaBuilder::table("posts")
        .attribute("title", AttributeType::Text)
        .attribute("content", AttributeType::Text)
        .attribute("published", AttributeType::Boolean)
        .foreign_key("author_id", AttributeType::Integer)
        .belongs_to(
            "author",
            Related::to("users")
                .pointing_own("author_id")
                .to_related("id"),
        )
        .has_many(
            "comments",
            Related::to("comments")
                .pointing_related("post_id")
                .to_own("id"),
        )
}

fn comments_schema() -> SchemaBuilder<'static> {
    SchemaBuilder::table("comments")
        .attribute("content", AttributeType::Text)
        .foreign_key("post_id", AttributeType::Integer)
        .foreign_key("author_id", AttributeType::Integer)
        .foreign_key("parent_id", AttributeType::Integer)
        .belongs_to(
            "post",
            Related::to("posts")
                .pointing_own("post_id")
                .to_related("id"),
        )
        .belongs_to(
            "author",
            Related::to("users")
                .pointing_own("author_id")
                .to_related("id"),
        )
        .belongs_to(
            "parent",
            Related::to("comments")
                .pointing_own("parent_id")
                .to_related("id"),
        )
        .has_many(
            "replies",
            Related::to("comments")
                .pointing_related("parent_id")
                .to_own("id"),
        )
}

fn tags_schema() -> SchemaBuilder<'static> {
    SchemaBuilder::table("tags").attribute("name", AttributeType::Text)
}

fn schemas() -> [SchemaBuilder<'static>; 5] {
    [
        users_schema(),
        profiles_schema(),
        posts_schema(),
        comments_schema(),
        tags_schema(),
    ]
}

fn with_database<F>(func: F) -> Result<(), Box<dyn Error>>
where
    F: FnOnce(&ConnectionManager<SqliteAdapter>) -> Result<(), Box<dyn Error>>,
{
    let manager: ConnectionManager<SqliteAdapter> =
        ConnectionManager::new(Registry::try_new(schemas())?, SqlitePool::memory()?);

    manager.acquire()?.execute_batch(
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

    func(&manager)?;

    Ok(())
}

fn seed_database(manager: &ConnectionManager<SqliteAdapter>) -> Result<(), Box<dyn Error>> {
    use Attribute::{Integer, Null};

    let connection = manager.acquire()?;

    // Create users
    let users_table = manager.table("users", &connection)?;
    for (i, (username, email)) in [
        ("alice", "alice@example.com"),
        ("bob", "bob@example.com"),
        ("charlie", "charlie@example.com"),
    ]
    .iter()
    .enumerate()
    {
        users_table.insert(
            Row::from_iter([
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
    let profiles_table = manager.table("profiles", &connection)?;
    for (id, user_id, bio, avatar) in [
        (1, 1, "Alice's bio", "https://example.com/alice.jpg"),
        (2, 2, "Bob's bio", "https://example.com/bob.jpg"),
        (3, 3, "Charlie's bio", "https://example.com/charlie.jpg"),
    ] {
        profiles_table.insert(
            Row::from_iter([
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
    let posts_table = manager.table("posts", &connection)?;
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
            Row::from_iter([
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
    let comments_table = manager.table("comments", &connection)?;
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
            Row::from_iter([
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
    let tags_table = manager.table("tags", &connection)?;
    for (id, name) in [(1, "rust"), (2, "programming"), (3, "web"), (4, "database")] {
        tags_table.insert(
            Row::from_iter([
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

    with_database(|manager| {
        seed_database(manager)?;

        let uri: Uri = "/users?include=posts.comments.replies.replies".parse()?;
        let schema = manager.registry().schema("users")?;
        let query_params = QueryParameters::parse(&uri, schema, manager.registry())?;

        let connection = manager.acquire()?;
        let mut collection = manager
            .table("users", &connection)?
            .query(&query_params)?
            .into_iter()
            .map(|row| Record::try_from_row(schema, row))
            .collect::<Result<Vec<_>, _>>()?;
        let included = DataLoader::new(manager, &connection)
            .load_for_collection(&mut collection, &query_params)?;
        let document = to_document(&collection, included, &uri, &DefaultUriGenerator::default())?;
        println!("{}", serde_json::to_string_pretty(&document)?);

        Ok(())
    })
}
