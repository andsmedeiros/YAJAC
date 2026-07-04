use crate::database::attributes::Identifier;
use crate::database::{
    adapters::SqliteAdapter,
    adapters::sqlite::Pool,
    attributes::{Attribute, Row},
    connection_manager::ConnectionManager,
    data_loader::DataLoader,
    query_parameters::QueryParameters,
    record::Record,
    registry::Registry,
    schema::{AttributeType, Related, SchemaBuilder, TableSchema},
    table::Table,
};
use crate::{core::to_document, http_wrappers::Uri, routing::DefaultUriGenerator};
use serde_json::{Value, json};
use std::error::Error;

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

fn schema<'sch>(
    manager: &'sch ConnectionManager<SqliteAdapter>,
    name: &str,
) -> &'sch TableSchema<'sch> {
    manager
        .registry()
        .schema(name)
        .expect("schema is registered")
}

fn with_database<F>(func: F) -> Result<(), Box<dyn Error>>
where
    F: FnOnce(&ConnectionManager<SqliteAdapter>) -> Result<(), Box<dyn Error>>,
{
    let manager: ConnectionManager<SqliteAdapter> =
        ConnectionManager::new(Registry::try_new(schemas())?, Pool::memory()?);

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
            &QueryParameters::new(schema(manager, "users")),
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
            &QueryParameters::new(schema(manager, "profiles")),
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
            &QueryParameters::new(schema(manager, "posts")),
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
            &QueryParameters::new(schema(manager, "comments")),
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
            &QueryParameters::new(schema(manager, "tags")),
        )?;
    }

    Ok(())
}

fn record_document<'a: 'b, 'b>(
    manager: &'b ConnectionManager<'a, SqliteAdapter>,
    model: &str,
    id: Identifier,
    uri_str: &str,
) -> Result<Value, Box<dyn Error>> {
    let uri: Uri = uri_str.parse()?;

    let connection = manager.acquire()?;
    let table = manager.table(model, &connection)?;
    let query_parameters = QueryParameters::parse(&uri, table.schema(), manager.registry())?;
    let row = table.find(id, &query_parameters)?;
    let mut record = Record::try_from_row(table.schema(), row)?;
    let loader = DataLoader::new(manager, &connection);
    let included = loader.load_for_record(&mut record, &query_parameters)?;
    let document = to_document(&record, included, &uri, &DefaultUriGenerator::default())?;

    Ok(serde_json::to_value(&document)?)
}

fn collection_document<'a: 'b, 'b>(
    manager: &'b ConnectionManager<'a, SqliteAdapter>,
    model: &str,
    uri_str: &str,
) -> Result<Value, Box<dyn Error>> {
    let uri: Uri = uri_str.parse()?;
    let connection = manager.acquire()?;
    let table = manager.table(model, &connection)?;
    let query_parameters = QueryParameters::parse(&uri, table.schema(), manager.registry())?;
    let mut collection = table
        .query(&query_parameters)?
        .into_iter()
        .map(|row| Record::try_from_row(table.schema(), row))
        .collect::<Result<Vec<_>, _>>()?;
    let loader = DataLoader::new(manager, &connection);
    let included = loader.load_for_collection(&mut collection, &query_parameters)?;
    let document = to_document(&collection, included, &uri, &DefaultUriGenerator::default())?;

    Ok(serde_json::to_value(&document)?)
}

#[test]
fn test_sparse_fieldset_only_username() -> Result<(), Box<dyn Error>> {
    with_database(|manager| {
        seed_database(manager)?;

        let doc = record_document(
            manager,
            "users",
            Identifier::Integer(1),
            "/users/1?fields[users]=username",
        )?;

        let data = &doc["data"];
        assert_eq!(data["type"], "users");
        assert_eq!(data["id"], "1");
        assert_eq!(data["attributes"]["username"], "alice");
        assert!(
            data["attributes"].get("email").is_none(),
            "email should not be present"
        );
        assert_eq!(
            data["relationships"],
            json!({}),
            "no relationships requested"
        );

        Ok(())
    })
}

#[test]
fn test_single_level_include_posts() -> Result<(), Box<dyn Error>> {
    with_database(|manager| {
        seed_database(manager)?;

        let doc = record_document(
            manager,
            "users",
            Identifier::Integer(1),
            "/users/1?include=posts",
        )?;

        let data = &doc["data"];
        assert!(
            data["relationships"]["posts"].is_object(),
            "posts relationship should be present"
        );

        let included = doc["included"]
            .as_array()
            .ok_or("included should be array")?;
        let post_count = included.iter().filter(|r| r["type"] == "posts").count();
        assert_eq!(post_count, 2, "Alice should have 2 posts");

        // Verify post ids
        let post_ids: Vec<&str> = included
            .iter()
            .filter(|r| r["type"] == "posts")
            .filter_map(|r| r["id"].as_str())
            .collect();
        assert!(post_ids.contains(&"1"));
        assert!(post_ids.contains(&"2"));

        Ok(())
    })
}

#[test]
fn test_multi_level_include_with_sparse_fieldsets() -> Result<(), Box<dyn Error>> {
    with_database(|manager| {
        seed_database(manager)?;

        let doc = record_document(
            manager,
            "users",
            Identifier::Integer(1),
            "/users/1?include=posts.comments&fields[users]=username&fields[posts]=title,comments",
        )?;

        let data = &doc["data"];
        assert_eq!(data["attributes"]["username"], "alice");
        assert_eq!(data["attributes"]["email"], Value::Null);
        assert_eq!(data["relationships"]["posts"], Value::Null);

        let included = doc["included"]
            .as_array()
            .ok_or("included should be array")?;

        // Check posts have only title
        let posts: Vec<_> = included.iter().filter(|r| r["type"] == "posts").collect();
        assert_eq!(posts.len(), 2);
        for post in &posts {
            assert!(post["attributes"]["title"].is_string());
            assert!(
                post["attributes"].get("content").is_none(),
                "content not in sparse fieldset"
            );
            assert!(
                post["attributes"].get("author_id").is_none(),
                "author_id not in sparse fieldset"
            );
            assert!(post["relationships"]["comments"].is_object());
        }

        // Check comments are included
        let comments: Vec<_> = included
            .iter()
            .filter(|r| r["type"] == "comments")
            .collect();
        assert_eq!(comments.len(), 8);

        Ok(())
    })
}

#[test]
fn test_deep_four_level_include() -> Result<(), Box<dyn Error>> {
    with_database(|manager| {
        seed_database(manager)?;

        let doc = record_document(
            manager,
            "users",
            Identifier::Integer(1),
            "/users/1?include=posts.comments.replies.replies",
        )?;

        let included = doc["included"]
            .as_array()
            .ok_or("included should be array")?;

        // Verify we have all 4 levels
        // Level 1: posts
        assert!(included.iter().any(|r| r["type"] == "posts"));

        // Level 2: comments on posts (comment 1, 2)
        let comment1 = included
            .iter()
            .find(|r| r["type"] == "comments" && r["id"] == "1")
            .ok_or("comment 1 should exist")?;
        assert!(comment1["relationships"]["replies"].is_object());

        // Level 3: replies to comments (comment 3, 4 are replies to 1)
        let comment3 = included
            .iter()
            .find(|r| r["type"] == "comments" && r["id"] == "3")
            .ok_or("comment 3 should exist (reply to comment 1)")?;
        assert!(comment3["relationships"]["replies"].is_object());

        // Level 4: replies to replies (comment 5 is reply to 3)
        let comment5 = included
            .iter()
            .find(|r| r["type"] == "comments" && r["id"] == "5")
            .ok_or("comment 5 should exist (reply to comment 3)")?;
        assert!(comment5["relationships"]["replies"].is_object());

        // Level 5 would be here: comment 6 is reply to 5
        let _comment6 = included
            .iter()
            .find(|r| r["type"] == "comments" && r["id"] == "6")
            .ok_or("comment 6 should exist (reply to comment 5)")?;

        Ok(())
    })
}

#[test]
fn test_multiple_relationships_same_level() -> Result<(), Box<dyn Error>> {
    with_database(|manager| {
        seed_database(manager)?;

        let doc = record_document(
            manager,
            "users",
            Identifier::Integer(1),
            "/users/1?include=posts,comments,profile",
        )?;

        let data = &doc["data"];
        assert!(data["relationships"]["posts"].is_object());
        assert!(data["relationships"]["comments"].is_object());
        assert!(data["relationships"]["profile"].is_object());

        let included = doc["included"]
            .as_array()
            .ok_or("included should be array")?;
        assert!(included.iter().any(|r| r["type"] == "posts"));
        assert!(included.iter().any(|r| r["type"] == "comments"));
        assert!(included.iter().any(|r| r["type"] == "profiles"));

        // Verify counts
        let posts_count = included.iter().filter(|r| r["type"] == "posts").count();
        let comments_count = included.iter().filter(|r| r["type"] == "comments").count();
        let profiles_count = included.iter().filter(|r| r["type"] == "profiles").count();

        assert_eq!(posts_count, 2, "Alice has 2 posts");
        assert!(comments_count >= 3, "Alice has made multiple comments");
        assert_eq!(profiles_count, 1, "Alice has 1 profile");

        Ok(())
    })
}

#[test]
fn test_self_referential_comment_replies() -> Result<(), Box<dyn Error>> {
    with_database(|manager| {
        seed_database(manager)?;

        let doc = record_document(
            manager,
            "comments",
            Identifier::Integer(1),
            "/comments/1?include=replies,replies.replies",
        )?;

        let data = &doc["data"];
        assert_eq!(data["id"], "1");
        assert!(data["relationships"]["replies"].is_object());

        let included = doc["included"]
            .as_array()
            .ok_or("included should be array")?;

        // Comment 1 has replies: 3 and 4
        assert!(
            included.iter().any(|r| r["id"] == "3"),
            "comment 3 is a reply to 1"
        );
        assert!(
            included.iter().any(|r| r["id"] == "4"),
            "comment 4 is a reply to 1"
        );

        // Comment 3 has reply: 5
        assert!(
            included.iter().any(|r| r["id"] == "5"),
            "comment 5 is a reply to 3"
        );

        Ok(())
    })
}

#[test]
fn test_belongs_to_with_author() -> Result<(), Box<dyn Error>> {
    with_database(|manager| {
        seed_database(manager)?;

        let doc = record_document(
            manager,
            "posts",
            Identifier::Integer(1),
            "/posts/1?fields[posts]=title,author&include=author",
        )?;

        let data = &doc["data"];
        assert!(data["relationships"]["author"].is_object());
        assert!(data["relationships"]["author"]["data"].is_object());
        assert_eq!(data["relationships"]["author"]["data"]["id"], "1");
        assert_eq!(data["relationships"]["author"]["data"]["type"], "users");

        let included = doc["included"]
            .as_array()
            .ok_or("included should be array")?;

        let author = included
            .iter()
            .find(|r| r["type"] == "users" && r["id"] == "1")
            .ok_or("author should be in included")?;
        assert_eq!(author["attributes"]["username"], "alice");

        Ok(())
    })
}

#[test]
fn test_collection_with_includes() -> Result<(), Box<dyn Error>> {
    with_database(|manager| {
        seed_database(manager)?;

        let doc = collection_document(
            manager,
            "posts",
            "/posts?include=author,comments&fields[posts]=title",
        )?;

        let data = doc["data"].as_array().ok_or("data should be array")?;
        assert!(data.len() >= 3, "should have multiple posts");

        for post in data {
            assert!(post["attributes"]["title"].is_string());
            assert!(
                post["attributes"].get("content").is_none(),
                "content not in sparse fieldset"
            );
            assert_eq!(post["relationships"]["author"], Value::Null);
            assert_eq!(post["relationships"]["comments"], Value::Null);
        }

        let included = doc["included"]
            .as_array()
            .ok_or("included should be array")?;
        assert!(
            included.iter().any(|r| r["type"] == "users"),
            "authors should be included"
        );
        assert!(
            included.iter().any(|r| r["type"] == "comments"),
            "comments should be included"
        );

        Ok(())
    })
}

#[test]
fn test_has_one_relationship() -> Result<(), Box<dyn Error>> {
    with_database(|manager| {
        seed_database(manager)?;

        let doc = record_document(
            manager,
            "users",
            Identifier::Integer(2),
            "/users/2?include=profile&fields[users]=username",
        )?;

        let data = &doc["data"];
        assert_eq!(data["attributes"]["username"], "bob");
        assert_eq!(data["relationships"]["profile"], Value::Null);

        let included = doc["included"]
            .as_array()
            .ok_or("included should be array")?;
        let profile = included
            .iter()
            .find(|r| r["type"] == "profiles")
            .ok_or("profile should be included")?;
        assert_eq!(profile["attributes"]["bio"], "Bob's bio");
        assert_eq!(profile["relationships"]["user"]["data"]["id"], "2");

        Ok(())
    })
}

#[test]
fn test_belongs_to_relationship_in_included() -> Result<(), Box<dyn Error>> {
    with_database(|manager| {
        seed_database(manager)?;

        let doc = record_document(
            manager,
            "users",
            Identifier::Integer(1),
            "/users/1?include=posts.author",
        )?;

        let included = doc["included"]
            .as_array()
            .ok_or("included should be array")?;

        // Find a post
        let post = included
            .iter()
            .find(|r| r["type"] == "posts")
            .ok_or("should have posts")?;

        // Post should have author relationship
        assert!(post["relationships"]["author"].is_object());
        assert_eq!(post["relationships"]["author"]["data"]["id"], "1");

        assert_eq!(doc["data"]["type"], "users");
        assert_eq!(doc["data"]["id"], "1");

        Ok(())
    })
}

#[test]
fn test_nested_belongs_to_chain() -> Result<(), Box<dyn Error>> {
    with_database(|manager| {
        seed_database(manager)?;

        let doc = record_document(
            manager,
            "comments",
            Identifier::Integer(9),
            "/comments/9?include=post.author",
        )?;

        let data = &doc["data"];
        assert_eq!(data["id"], "9");

        let included = doc["included"]
            .as_array()
            .ok_or("included should be array")?;

        // Should include post 3
        let post = included
            .iter()
            .find(|r| r["type"] == "posts" && r["id"] == "3")
            .ok_or("post 3 should be included")?;
        assert!(post["relationships"]["author"].is_object());

        // Should include user 2 (Bob, author of post 3)
        let author = included
            .iter()
            .find(|r| r["type"] == "users" && r["id"] == "2")
            .ok_or("user 2 should be included")?;
        assert_eq!(author["attributes"]["username"], "bob");

        Ok(())
    })
}

#[test]
fn test_sparse_fieldset_excludes_relationships_not_requested() -> Result<(), Box<dyn Error>> {
    with_database(|manager| {
        seed_database(manager)?;

        // Request only username, which means posts relationship should NOT appear
        let doc = record_document(
            manager,
            "users",
            Identifier::Integer(1),
            "/users/1?fields[users]=username&include=posts",
        )?;

        let data = &doc["data"];
        assert_eq!(data["attributes"]["username"], "alice");

        // posts is not in fields[users], so relationship should not be in primary data
        assert_eq!(data["relationships"], json!({}));

        // But posts should still be in included section
        let included = doc["included"]
            .as_array()
            .ok_or("included should be array")?;
        assert!(included.iter().any(|r| r["type"] == "posts"));

        Ok(())
    })
}

#[test]
fn test_relationship_without_include() -> Result<(), Box<dyn Error>> {
    with_database(|manager| {
        seed_database(manager)?;

        // Request posts relationship in fieldset but don't include it
        let doc = record_document(
            manager,
            "users",
            Identifier::Integer(1),
            "/users/1?fields[users]=username,posts",
        )?;

        let data = &doc["data"];
        assert_eq!(data["attributes"]["username"], "alice");

        // posts relationship should be present with data
        assert!(data["relationships"]["posts"].is_object());
        assert!(data["relationships"]["posts"]["data"].is_array());

        let post_refs = data["relationships"]["posts"]["data"]
            .as_array()
            .ok_or("post refs should be array")?;
        assert_eq!(post_refs.len(), 2);

        // But included should be empty since we didn't request include
        let included = doc["included"]
            .as_array()
            .ok_or("included should be array")?;
        assert_eq!(included.len(), 0, "no resources should be included");

        Ok(())
    })
}
