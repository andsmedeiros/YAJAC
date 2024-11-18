use crate::spec::{
    identifier::Identifier,
    resource::Resource
};
use std::{
    borrow::Borrow, 
    collections::HashMap
};

pub struct Cache<'a> {
    resources: Vec<Resource>,
    index: HashMap<Identifier, &'a Resource>
}

impl<'a> Default for Cache<'a> {
    fn default() -> Self {
        Cache { resources: Vec::new(), index: HashMap::new() }
    }
}

impl<'a> Cache<'a> {
    pub fn new() -> Self { 
        Self::default() 
    }

    pub fn has(&self, identifier: impl Borrow<Identifier>) -> bool {
        self.index.contains_key(identifier.borrow())
    }

    pub fn get(&self, identifier: impl Borrow<Identifier>) -> Option<&Resource> {
        self.index.get(identifier.borrow())?.clone().into()
    }

    pub fn register(&'a mut self, identifier: impl Borrow<Identifier>, resource: Resource) -> &Resource {
        self.resources.push(resource);
        let resource = self.resources.last().unwrap();
        self.index.insert(identifier.borrow().clone(), resource);

        resource
    }
}