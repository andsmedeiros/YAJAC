use std::{
    cell::RefCell,
    collections::HashMap, 
    marker, 
    rc::{Rc, Weak}
};
use serde::{Serialize, Deserialize};
use serde_json::{Map, Value};
use ::json_api::{
    resourceful::{RelatedData, Resourceful},
    spec::identifier::Identifier,
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

    pub fn find(&self, id: IdType) -> Option<T> {
        self.data.iter()
            .find(|r| r.model.get_id() == id)
            .cloned()
            .map(|r| r.model)
    }
}

struct Database {
    users: Table<User>,
    posts: Table<Post>,
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
    user_id: String,
}

impl Identifiable for Post {
    fn get_id(&self) -> String {
        self.id.clone()
    }
}

// impl Resourceful for Record<User> {
//     fn get_type(&self) -> &'static str {
//         "users"
//     }

//     fn get_identifier(&self) -> Identifier {
//         Identifier::Existing {
//             kind: self.get_type().to_string(),
//             id: self.model.id.clone(),
//         }
//     }

//     fn get_attributes(&self) -> Option<HashMap<String, Value>> {
//         json_api::extract!(self.model, [name, age, active])
//     }

//     fn get_relationships(&self) -> Option<HashMap<String, RelatedData>> {
//         Relationships::new([
//             ("posts", )
//         ])
//     }
// }

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
                user_id: "67e55044-10b1-426f-9247-bb680e5fe0c8".to_string(),
            },
            Post {
                id: "d22bcb69-e162-40d7-8e77-821e1be1e63e".to_string(),
                title: "This is another post".to_string(),
                content: "This is the content of the other post".to_string(),
                user_id: "67e55044-10b1-426f-9247-bb680e5fe0c8".to_string(),
            },
            Post {
                id: "e2982a7a-9901-4ff9-a5ff-204ded2c21d5".to_string(),
                title: "This is another post".to_string(),
                content: "This is the content of the other post".to_string(),
                user_id: "1cc61e1e-b862-4b3b-88a9-422ae3077145".to_string(),
            }
        ]
    );

    // let resource = ;

    // let resources = vec![
    //     resource,
    // ];
    // println!(
    //     "resources:\n{}",
    //     serde_json::to_string_pretty(&resources
    //         .into_iter()
    //         .map(|r| r.into_resource())
    //         .collect::<Vec<Resource>>()
    //     ).unwrap()
    // );
    // println!("document:\n{}", serde_json::to_string_pretty(&resource.into_document()).unwrap());
    // println!("document:\n{}", serde_json::to_string_pretty(&Document::from(resources)).unwrap());

    // println!("{:#?}", Document::from(USERS));
}