use yajac::{
    core::to_document,
    http_wrappers::Uri,
    routing::DefaultUriGenerator,
};
use yajac::database::attributes::{Attribute, Attributes};
use yajac::database::data_loader::DataLoader;
use yajac::database::schema::{AttributeType, RelatedTable, Relationship, RelationshipColumns};
use yajac::database::table::Table;


fn main() -> Result<(), Box<dyn std::error::Error>> {
    use yajac::database::{
        adapters::SqliteAdapter,
        attributes::Attribute,
        query_parameters::QueryParameters,
        registry::Registry,
        schema::{TableSchema, AttributeType, Relationship},
    };
    use rusqlite::Connection;

    colog::init();

    let connection = Connection::open(":memory:")?;
    connection.execute_batch(
        "
        CREATE TABLE users (
            id INTEGER PRIMARY KEY,
            name TEXT,
            age INTEGER,
            active BOOLEAN
        );
        CREATE TABLE posts (
            id INTEGER PRIMARY KEY,
            title TEXT,
            content TEXT,
            author_id INTEGER NON NULL,
            FOREIGN KEY(author_id) REFERENCES users(id)
        );
        "
    )?;

    let registry = Registry::<SqliteAdapter>::try_new(connection, &[
        &TableSchema {
            name: "users",
            columns: &[
                ("id", AttributeType::Integer),
                ("name", AttributeType::Text),
                ("age", AttributeType::Integer),
                ("active", AttributeType::Boolean)
            ],
            relationships: &[
                ("posts", Relationship::HasMany(RelatedTable {
                    table: "posts",
                    columns: RelationshipColumns { related: "author_id", own: "id" }
                })),
            ],
            text_index: false
        },
        &TableSchema {
            name: "posts",
            columns: &[
                ("id", AttributeType::Integer),
                ("title", AttributeType::Text),
                ("content", AttributeType::Text),
                ("author_id", AttributeType::Integer)
            ],
            relationships: &[
                ("author", Relationship::BelongsTo(RelatedTable {
                    table: "users",
                    columns: RelationshipColumns { related: "id", own: "author_id" }
                }))
            ],
            text_index: false
        }
    ])?;

    let users_table = registry.table("users")?;

    let users = vec![
        [
            ("id".to_string(), Attribute::Integer(1)),
            ("name".to_string(), Attribute::Text("André Medeiros".to_string())),
            ("age".to_string(), Attribute::Integer(36)),
            ("active".to_string(), Attribute::Boolean(true))
        ],
        [
            ("id".to_string(), Attribute::Integer(2)),
            ("name".to_string(), Attribute::Text("Gustavo Godoy".to_string())),
            ("age".to_string(), Attribute::Integer(43)),
            ("active".to_string(), Attribute::Boolean(false))
        ]
    ];

    for user in users {
        let attributes = Attributes::from_iter(user);
        let record = users_table.insert(attributes, &QueryParameters::default())?;

        println!("Inserted user: {:?}", record.attributes);
    }

    let posts_table = registry.table("posts")?;
    let posts = vec![
        [
            ("id".to_string(), Attribute::Integer(1)),
            ("title".to_string(), Attribute::Text("This is a post".to_string())),
            ("content".to_string(), Attribute::Text("This is the content of the post".to_string())),
            ("author_id".to_string(), Attribute::Integer(1)),
        ],
        [
            ("id".to_string(), Attribute::Integer(2)),
            ("title".to_string(), Attribute::Text("This is another post".to_string())),
            ("content".to_string(), Attribute::Text("This is the content of the other post".to_string())),
            ("author_id".to_string(), Attribute::Integer(1)),
        ],
        [
            ("id".to_string(), Attribute::Integer(3)),
            ("title".to_string(), Attribute::Null),
            ("content".to_string(), Attribute::Text("This post has no title!".to_string())),
            ("author_id".to_string(), Attribute::Integer(2)),
        ]
    ];

    for post in posts {
        let attributes = Attributes::from_iter(post);
        let record = posts_table.insert(attributes, &QueryParameters::default())?;

        println!("Inserted post: {:?}", record.attributes);
    }

    let uri: Uri = "/users/1?fields[users]=name,active&include=posts&fields[posts]=title,author".parse()?;
    let query_params = QueryParameters::parse(&uri)?;

    // let mut collection = users_table.query(&query_params)?;
    // let included = DataLoader::new(&registry)
    //     .load_for_collection(&mut collection, &query_params)?;
    let mut record = users_table.find(1, &query_params)?;
    let included = DataLoader::new(&registry)
        .load_for_record(&mut record, &query_params)?;
    let document =
        to_document(&record, included, uri, &query_params, &DefaultUriGenerator::default())?;
    println!("{}", serde_json::to_string_pretty(&document).unwrap());

    Ok(())
}
