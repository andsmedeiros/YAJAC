use std::{
    cell::RefCell,
    marker, 
    rc::{Rc, Weak}
};
use serde::{Serialize, Deserialize};
use yajac::{
    adapter::{Adapter, Context, DefaultUriGenerator, Parameters, UriGenerator},
    http_wrappers::Uri,
    resourceful::{Attributes, Relationships, Resourceful,},
    spec::identifier::Identifier,
    extract_filtered
};

trait Identifiable<IdType=String> {
    fn get_id(&self) -> IdType;
}

#[derive(Clone)]
struct Record<M> {
    model: M,
    database: Weak<RefCell<Database>>
}

struct Table<T, IdType=String> 
where 
    T: Identifiable<IdType> + Clone,
    IdType: PartialEq
{
    data: Vec<Record<T>>,
    phantom: marker::PhantomData<IdType>
}

impl<T, IdType> Table<T, IdType> 
where
    T: Identifiable<IdType> + Clone,
    IdType: PartialEq
{
    pub fn new() -> Self {
        Self { data: vec![], phantom: marker::PhantomData }
    }

    pub fn all(&self) -> Vec<T> {
        self.data.iter().map(|r| r.model.clone()).collect()
    }

    pub fn find(&self, id: &IdType) -> Option<T> {
        self.data.iter()
            .find(|r| r.model.get_id() == *id)
            .cloned()
            .map(|r| r.model)
    }
}

struct Database {
    pub users: Table<User>,
    pub posts: Table<Post>,
}

impl Database {
    pub fn new(users: Vec<User>, posts: Vec<Post>) -> Rc<RefCell<Self>> {
        let database = Rc::new(RefCell::new(Database { 
            users: Table::new(),
            posts: Table::new()
        }));

        let mut users = users.into_iter()
            .map(|model| Record { model, database: Rc::downgrade(&database) })
            .collect();

        let mut posts = posts.into_iter()
            .map(|model| Record { model, database: Rc::downgrade(&database) })
            .collect();

        std::mem::swap(&mut database.borrow_mut().users.data, &mut users);
        std::mem::swap(&mut database.borrow_mut().posts.data, &mut posts);

        database
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct User {
    id: String,
    name: String,
    age: u8,
    active: bool,
}

impl Identifiable for User {
    fn get_id(&self) -> String {
        self.id.clone()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Post {
    id: String,
    title: String,
    content: String,
    author_id: String,
}

impl Identifiable for Post {
    fn get_id(&self) -> String {
        self.id.clone()
    }
}

impl Resourceful for Record<User> {
    fn kind(&self) -> &'static str {
        "users"
    }

    fn identifier(&self) -> Identifier {
        Identifier::Existing {
            kind: self.kind().to_string(),
            id: self.model.id.clone(),
        }
    }

    fn attributes<G: UriGenerator>(&self, context: &Context<G>) -> Option<Attributes> {
        extract_filtered!(self.model, [name, age, active], context.fields_for(self.kind()))
    }

    fn relationships<G: UriGenerator>(&self, context: &mut Context<G>)
        -> Option<Relationships>
    {
        let posts = self.database.upgrade()?.borrow()
            .posts.all()
            .into_iter()
            .filter_map(|model|
                if model.author_id == self.model.id {
                    Some(Record { model, database: self.database.clone() })
                } else {
                    None
                }
            );

        Relationships::from([
            context.link_many("posts", posts),
        ]).into()
    }
}

impl Resourceful for Record<Post> {
    fn kind(&self) -> &'static str { "posts" }
    fn identifier(&self) -> Identifier {
        Identifier::Existing {
            kind: self.kind().to_string(),
            id: self.model.id.clone(),
        }
    }

    fn attributes<G: UriGenerator>(&self, context: &Context<G>) -> Option<Attributes> {
        extract_filtered!(self.model, [title, content], context.fields_for(self.kind()))
    }

    fn relationships<G: UriGenerator>(&self, context: &mut Context<G>)
        -> Option<Relationships>
    {
        let author = self.database.upgrade()?.borrow()
            .users
            .find(&self.model.author_id)
            .map(|model| Record { model, database: self.database.clone() });

        Relationships::from([
            context.link_one("author", author),
            context.link_one("always_empty", Option::<Record<Post>>::None)
        ]).into()
    }
}

fn main() {
    let database = Database::new(
        vec![
            User {
                id: "67e55044-10b1-426f-9247-bb680e5fe0c8".to_string(),
                name: "André Medeiros".to_string(),
                age: 35,
                active: true
            },
            User {
                id: "1cc61e1e-b862-4b3b-88a9-422ae3077145".to_string(),
                name: "Gustavo Godoy".to_string(),
                age: 42,
                active: false
            }
        ],
        vec![
            Post {
                id: "5fc5ecc8-e286-4b04-9dd6-38306e7b5742".to_string(),
                title: "This is a post".to_string(),
                content: "This is the content of the post".to_string(),
                author_id: "67e55044-10b1-426f-9247-bb680e5fe0c8".to_string(),
            },
            Post {
                id: "d22bcb69-e162-40d7-8e77-821e1be1e63e".to_string(),
                title: "This is another post".to_string(),
                content: "This is the content of the other post".to_string(),
                author_id: "67e55044-10b1-426f-9247-bb680e5fe0c8".to_string(),
            },
            Post {
                id: "e2982a7a-9901-4ff9-a5ff-204ded2c21d5".to_string(),
                title: "This is another post".to_string(),
                content: "This is the content of the other post".to_string(),
                author_id: "1cc61e1e-b862-4b3b-88a9-422ae3077145".to_string(),
            }
        ]
    );

    let uri: Uri = "/users/67e55044-10b1-426f-9247-bb680e5fe0c8?fields[users]=name,active,non-existent-field&include=posts,always_empty&fields[posts]=title".parse().unwrap();
    let generator = DefaultUriGenerator::default();
    let adapter = Adapter::new(uri, generator);
    let user = database.borrow().users.find(&"67e55044-10b1-426f-9247-bb680e5fe0c8".into()).unwrap();
    let record = Record { model: user, database: Rc::downgrade(&database) };
    let document = adapter.into_resource_document(&record);

    println!("{}", serde_json::to_string_pretty(&document).unwrap());
}