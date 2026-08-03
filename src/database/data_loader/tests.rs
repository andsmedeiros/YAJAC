use crate::database::attributes::Identifier;
use crate::database::relationships::Relationship;
use crate::database::{
    adapters::SqliteAdapter,
    adapters::sqlite::Pool,
    attributes::{Attribute, Row},
    connection_manager::ConnectionManager,
    data_loader::DataLoader,
    query_parameters::QueryParameters,
    record::Record,
    registry::Registry,
    schema::{AttributeType, Related, Schema, SchemaBuilder},
    table::Table,
};
use crate::http_wrappers::Uri;
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

fn schema<'sch>(manager: &'sch ConnectionManager<SqliteAdapter>, name: &str) -> &'sch Schema<'sch> {
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
                ("id", Attribute::Integer((i + 1) as i64)),
                ("username", Attribute::Text(username.to_string())),
                ("email", Attribute::Text(email.to_string())),
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
                ("id", Attribute::Integer(id)),
                ("user_id", Attribute::Integer(user_id)),
                ("bio", Attribute::Text(bio.to_string())),
                ("avatar_url", Attribute::Text(avatar.to_string())),
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
                ("id", Attribute::Integer(id)),
                ("author_id", Attribute::Integer(author_id)),
                ("title", Attribute::Text(title.to_string())),
                ("content", Attribute::Text(content.to_string())),
                ("published", Attribute::Boolean(published)),
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
                ("id", Attribute::Integer(id)),
                ("post_id", Attribute::Integer(post_id)),
                ("author_id", Attribute::Integer(author_id)),
                ("parent_id", parent_id),
                ("content", Attribute::Text(content.to_string())),
            ]),
            &QueryParameters::new(schema(manager, "comments")),
        )?;
    }

    // Create tags
    let tags_table = manager.table("tags", &connection)?;
    for (id, name) in [(1, "rust"), (2, "programming"), (3, "web"), (4, "database")] {
        tags_table.insert(
            Row::from_iter([
                ("id", Attribute::Integer(id)),
                ("name", Attribute::Text(name.to_string())),
            ]),
            &QueryParameters::new(schema(manager, "tags")),
        )?;
    }

    Ok(())
}

// Loads a record and the included set the DataLoader gathers for the request.
fn load_record<'sch>(
    manager: &'sch ConnectionManager<'sch, SqliteAdapter>,
    model: &str,
    id: Identifier,
    uri_str: &str,
) -> Result<(Record<'sch>, Vec<Record<'sch>>), Box<dyn Error>> {
    let uri: Uri = uri_str.parse()?;
    let connection = manager.acquire()?;
    let table = manager.table(model, &connection)?;
    let query_parameters = QueryParameters::parse(&uri, table.schema(), manager.registry())?;
    let row = table.find(id, &query_parameters)?;
    let mut record = Record::try_from_row(table.schema(), row)?;
    let included =
        DataLoader::new(manager, &connection).load_for_record(&mut record, &query_parameters)?;

    Ok((record, included))
}

// Loads a collection and the included set the DataLoader gathers for the request.
fn load_collection<'sch>(
    manager: &'sch ConnectionManager<'sch, SqliteAdapter>,
    model: &str,
    uri_str: &str,
) -> Result<(Vec<Record<'sch>>, Vec<Record<'sch>>), Box<dyn Error>> {
    let uri: Uri = uri_str.parse()?;
    let connection = manager.acquire()?;
    let table = manager.table(model, &connection)?;
    let query_parameters = QueryParameters::parse(&uri, table.schema(), manager.registry())?;
    let mut collection = table
        .query(&query_parameters)?
        .into_iter()
        .map(|row| Record::try_from_row(table.schema(), row))
        .collect::<Result<Vec<_>, _>>()?;
    let included = DataLoader::new(manager, &connection)
        .load_for_collection(&mut collection, &query_parameters)?;

    Ok((collection, included))
}

// The record's integer id.
fn id(record: &Record) -> Option<i64> {
    record
        .get_id()
        .and_then(|identifier| identifier.to_i64().ok())
}

// The record's `name` attribute, if present and textual.
fn text<'req>(record: &'req Record, name: &str) -> Option<&'req str> {
    match record.get(name) {
        Some(Attribute::Text(value)) => Some(value.as_str()),
        _ => None,
    }
}

// The included records of `kind`, in load order.
fn of_kind<'sch, 'req>(included: &'req [Record<'sch>], kind: &str) -> Vec<&'req Record<'sch>> {
    included
        .iter()
        .filter(|record| record.kind() == kind)
        .collect()
}

// The included record of `kind` with integer id `target`.
fn find<'sch, 'req>(
    included: &'req [Record<'sch>],
    kind: &str,
    target: i64,
) -> Option<&'req Record<'sch>> {
    included
        .iter()
        .find(|record| record.kind() == kind && id(record) == Some(target))
}

// The integer id a to-one relationship points at.
fn to_one(record: &Record, name: &str) -> Option<i64> {
    match record.get_related(name) {
        Some(Relationship::BelongsTo(identifier)) | Some(Relationship::HasOne(identifier)) => {
            identifier.to_i64().ok()
        }
        _ => None,
    }
}

// The identifiers a to-many relationship points at.
fn to_many<'req>(record: &'req Record, name: &str) -> Option<&'req [Identifier]> {
    match record.get_related(name) {
        Some(Relationship::HasMany(identifiers)) => Some(identifiers.as_slice()),
        _ => None,
    }
}

#[test]
fn test_sparse_fieldset_only_username() -> Result<(), Box<dyn Error>> {
    with_database(|manager| {
        seed_database(manager)?;

        let (record, _included) = load_record(
            manager,
            "users",
            Identifier::Integer(1),
            "/users/1?fields[users]=username",
        )?;

        assert_eq!(record.kind(), "users");
        assert_eq!(id(&record), Some(1));
        assert_eq!(text(&record, "username"), Some("alice"));
        assert!(record.get("email").is_none(), "email should not be present");
        assert!(
            record.relationships.is_empty(),
            "no relationships requested"
        );

        Ok(())
    })
}

#[test]
fn test_single_level_include_posts() -> Result<(), Box<dyn Error>> {
    with_database(|manager| {
        seed_database(manager)?;

        let (record, included) = load_record(
            manager,
            "users",
            Identifier::Integer(1),
            "/users/1?include=posts",
        )?;

        assert!(
            record.get_related("posts").is_some(),
            "posts relationship should be present"
        );

        let mut post_ids: Vec<i64> = of_kind(&included, "posts")
            .iter()
            .filter_map(|&post| id(post))
            .collect();
        post_ids.sort();
        assert_eq!(post_ids, vec![1, 2], "Alice's posts are exactly 1 and 2");

        Ok(())
    })
}

#[test]
fn test_multi_level_include_with_sparse_fieldsets() -> Result<(), Box<dyn Error>> {
    with_database(|manager| {
        seed_database(manager)?;

        let (record, included) = load_record(
            manager,
            "users",
            Identifier::Integer(1),
            "/users/1?include=posts.comments&fields[users]=username&fields[posts]=title,comments",
        )?;

        assert_eq!(text(&record, "username"), Some("alice"));
        assert!(record.get("email").is_none());
        assert!(
            record.get_related("posts").is_none(),
            "posts is excluded from the users fieldset"
        );

        // Posts carry only their title, and their comments relationship.
        let posts = of_kind(&included, "posts");
        let mut post_ids: Vec<i64> = posts.iter().filter_map(|&post| id(post)).collect();
        post_ids.sort();
        assert_eq!(post_ids, vec![1, 2]);
        for &post in &posts {
            assert!(text(post, "title").is_some());
            assert!(
                post.get("content").is_none(),
                "content not in sparse fieldset"
            );
            assert!(
                post.get_related("comments").is_some(),
                "comments was solicited"
            );
            assert!(
                post.get_related("author").is_none(),
                "author was not solicited, so it must not be loaded"
            );
        }

        // Every comment on those two posts is included.
        let mut comment_ids: Vec<i64> = of_kind(&included, "comments")
            .iter()
            .filter_map(|&comment| id(comment))
            .collect();
        comment_ids.sort();
        assert_eq!(comment_ids, vec![1, 2, 3, 4, 5, 6, 7, 8]);

        Ok(())
    })
}

#[test]
fn test_deep_four_level_include() -> Result<(), Box<dyn Error>> {
    with_database(|manager| {
        seed_database(manager)?;

        let (_record, included) = load_record(
            manager,
            "users",
            Identifier::Integer(1),
            "/users/1?include=posts.comments.replies.replies",
        )?;

        // Level 1: posts.
        assert!(!of_kind(&included, "posts").is_empty());

        // Level 2: comment 1 (on a post) carries its replies relationship.
        let comment1 = find(&included, "comments", 1).ok_or("comment 1 should exist")?;
        assert!(comment1.get_related("replies").is_some());

        // Level 3: comment 3 (a reply to 1) carries its replies.
        let comment3 = find(&included, "comments", 3).ok_or("comment 3 should exist")?;
        assert!(comment3.get_related("replies").is_some());

        // Level 4: comment 5 (a reply to 3) carries its replies.
        let comment5 = find(&included, "comments", 5).ok_or("comment 5 should exist")?;
        assert!(comment5.get_related("replies").is_some());

        // Level 5: comment 6 (a reply to 5) is loaded.
        find(&included, "comments", 6).ok_or("comment 6 should exist")?;

        Ok(())
    })
}

#[test]
fn test_multiple_relationships_same_level() -> Result<(), Box<dyn Error>> {
    with_database(|manager| {
        seed_database(manager)?;

        let (record, included) = load_record(
            manager,
            "users",
            Identifier::Integer(1),
            "/users/1?include=posts,comments,profile",
        )?;

        assert!(record.get_related("posts").is_some());
        assert!(record.get_related("comments").is_some());
        assert!(record.get_related("profile").is_some());

        let mut post_ids: Vec<i64> = of_kind(&included, "posts")
            .iter()
            .filter_map(|&post| id(post))
            .collect();
        post_ids.sort();
        assert_eq!(post_ids, vec![1, 2], "Alice's posts");

        let mut comment_ids: Vec<i64> = of_kind(&included, "comments")
            .iter()
            .filter_map(|&comment| id(comment))
            .collect();
        comment_ids.sort();
        assert_eq!(comment_ids, vec![3, 6, 9], "the comments Alice authored");

        let profile_ids: Vec<i64> = of_kind(&included, "profiles")
            .iter()
            .filter_map(|&profile| id(profile))
            .collect();
        assert_eq!(profile_ids, vec![1], "Alice's profile");

        Ok(())
    })
}

#[test]
fn test_self_referential_comment_replies() -> Result<(), Box<dyn Error>> {
    with_database(|manager| {
        seed_database(manager)?;

        let (record, included) = load_record(
            manager,
            "comments",
            Identifier::Integer(1),
            "/comments/1?include=replies,replies.replies",
        )?;

        assert_eq!(id(&record), Some(1));
        assert!(record.get_related("replies").is_some());

        // Comment 1's replies are 3 and 4; comment 3's reply is 5.
        let mut reply_ids: Vec<i64> = of_kind(&included, "comments")
            .iter()
            .filter_map(|&comment| id(comment))
            .collect();
        reply_ids.sort();
        assert_eq!(reply_ids, vec![3, 4, 5]);

        Ok(())
    })
}

#[test]
fn test_belongs_to_with_author() -> Result<(), Box<dyn Error>> {
    with_database(|manager| {
        seed_database(manager)?;

        let (record, included) = load_record(
            manager,
            "posts",
            Identifier::Integer(1),
            "/posts/1?fields[posts]=title,author&include=author",
        )?;

        assert_eq!(to_one(&record, "author"), Some(1));

        let author_ids: Vec<i64> = of_kind(&included, "users")
            .iter()
            .filter_map(|&author| id(author))
            .collect();
        assert_eq!(author_ids, vec![1], "only the author is included");
        let author = find(&included, "users", 1).ok_or("author should be in included")?;
        assert_eq!(text(author, "username"), Some("alice"));

        Ok(())
    })
}

#[test]
fn test_collection_with_includes() -> Result<(), Box<dyn Error>> {
    with_database(|manager| {
        seed_database(manager)?;

        let (collection, included) = load_collection(
            manager,
            "posts",
            "/posts?include=author,comments&fields[posts]=title",
        )?;

        let mut post_ids: Vec<i64> = collection.iter().filter_map(id).collect();
        post_ids.sort();
        assert_eq!(post_ids, vec![1, 2, 3, 4, 5]);
        for post in &collection {
            assert!(text(post, "title").is_some());
            assert!(
                post.get("content").is_none(),
                "content not in sparse fieldset"
            );
            assert!(
                post.get_related("author").is_none(),
                "author excluded from fieldset"
            );
            assert!(
                post.get_related("comments").is_none(),
                "comments excluded from fieldset"
            );
        }

        let mut author_ids: Vec<i64> = of_kind(&included, "users")
            .iter()
            .filter_map(|&author| id(author))
            .collect();
        author_ids.sort();
        assert_eq!(
            author_ids,
            vec![1, 2, 3],
            "the distinct authors of every post"
        );

        let mut comment_ids: Vec<i64> = of_kind(&included, "comments")
            .iter()
            .filter_map(|&comment| id(comment))
            .collect();
        comment_ids.sort();
        assert_eq!(
            comment_ids,
            (1..=11).collect::<Vec<_>>(),
            "every post's comments"
        );

        Ok(())
    })
}

#[test]
fn test_has_one_relationship() -> Result<(), Box<dyn Error>> {
    with_database(|manager| {
        seed_database(manager)?;

        let (record, included) = load_record(
            manager,
            "users",
            Identifier::Integer(2),
            "/users/2?include=profile&fields[users]=username",
        )?;

        assert_eq!(text(&record, "username"), Some("bob"));
        assert!(
            record.get_related("profile").is_none(),
            "profile excluded from the users fieldset"
        );

        let profile_ids: Vec<i64> = of_kind(&included, "profiles")
            .iter()
            .filter_map(|&profile| id(profile))
            .collect();
        assert_eq!(profile_ids, vec![2], "Bob's profile");
        let profile = find(&included, "profiles", 2).ok_or("profile should be included")?;
        assert_eq!(text(profile, "bio"), Some("Bob's bio"));
        assert_eq!(to_one(profile, "user"), Some(2));

        Ok(())
    })
}

#[test]
fn test_belongs_to_relationship_in_included() -> Result<(), Box<dyn Error>> {
    with_database(|manager| {
        seed_database(manager)?;

        let (record, included) = load_record(
            manager,
            "users",
            Identifier::Integer(1),
            "/users/1?include=posts.author",
        )?;

        assert_eq!(record.kind(), "users");
        assert_eq!(id(&record), Some(1));

        // Both of Alice's posts belong to her, so she is the only included author.
        let posts = of_kind(&included, "posts");
        assert!(!posts.is_empty(), "should have posts");
        for &post in &posts {
            assert_eq!(to_one(post, "author"), Some(1));
        }
        let author_ids: Vec<i64> = of_kind(&included, "users")
            .iter()
            .filter_map(|&author| id(author))
            .collect();
        assert_eq!(author_ids, vec![1]);

        Ok(())
    })
}

#[test]
fn test_nested_belongs_to_chain() -> Result<(), Box<dyn Error>> {
    with_database(|manager| {
        seed_database(manager)?;

        let (record, included) = load_record(
            manager,
            "comments",
            Identifier::Integer(9),
            "/comments/9?include=post.author",
        )?;

        assert_eq!(id(&record), Some(9));

        // comment 9 → post 3 → author bob (user 2).
        let post_ids: Vec<i64> = of_kind(&included, "posts")
            .iter()
            .filter_map(|&post| id(post))
            .collect();
        assert_eq!(post_ids, vec![3], "the comment's post");
        let post = find(&included, "posts", 3).ok_or("post 3 should be included")?;
        assert_eq!(to_one(post, "author"), Some(2));

        let author_ids: Vec<i64> = of_kind(&included, "users")
            .iter()
            .filter_map(|&author| id(author))
            .collect();
        assert_eq!(author_ids, vec![2], "the post's author");
        let author = find(&included, "users", 2).ok_or("user 2 should be included")?;
        assert_eq!(text(author, "username"), Some("bob"));

        Ok(())
    })
}

#[test]
fn test_sparse_fieldset_excludes_relationships_not_requested() -> Result<(), Box<dyn Error>> {
    with_database(|manager| {
        seed_database(manager)?;

        // Request only username, which means posts relationship should NOT appear
        let (record, included) = load_record(
            manager,
            "users",
            Identifier::Integer(1),
            "/users/1?fields[users]=username&include=posts",
        )?;

        assert_eq!(text(&record, "username"), Some("alice"));
        assert!(
            record.relationships.is_empty(),
            "posts is not in fields[users], so no relationship on the primary record"
        );

        // But posts still appear in the included set.
        let mut post_ids: Vec<i64> = of_kind(&included, "posts")
            .iter()
            .filter_map(|&post| id(post))
            .collect();
        post_ids.sort();
        assert_eq!(post_ids, vec![1, 2]);

        Ok(())
    })
}

#[test]
fn test_relationship_without_include() -> Result<(), Box<dyn Error>> {
    with_database(|manager| {
        seed_database(manager)?;

        // Request posts relationship in fieldset but don't include it
        let (record, included) = load_record(
            manager,
            "users",
            Identifier::Integer(1),
            "/users/1?fields[users]=username,posts",
        )?;

        assert_eq!(text(&record, "username"), Some("alice"));

        // posts is in the fieldset, so the relationship is present with its two references...
        let post_refs = to_many(&record, "posts").ok_or("posts relationship should be present")?;
        let mut post_ref_ids: Vec<i64> = post_refs
            .iter()
            .filter_map(|identifier| identifier.to_i64().ok())
            .collect();
        post_ref_ids.sort();
        assert_eq!(post_ref_ids, vec![1, 2]);

        // ...but nothing is included, since include was not requested.
        assert!(included.is_empty(), "no resources should be included");

        Ok(())
    })
}
