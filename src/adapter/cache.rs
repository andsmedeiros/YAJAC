use crate::spec::{
    identifier::Identifier,
    resource::Resource
};
use std::{
    borrow::Borrow, 
    collections::HashMap
};

pub struct Cache {
    index: HashMap<Identifier, Resource>
}

impl Default for Cache {
    fn default() -> Self {
        Cache { index: HashMap::new() }
    }
}

impl Cache {
    pub fn new() -> Self { 
        Self::default() 
    }

    pub fn has(&self, identifier: impl Borrow<Identifier>) -> bool {
        self.index.contains_key(identifier.borrow())
    }

    pub fn get(&self, identifier: impl Borrow<Identifier>) -> Option<&Resource> {
        Some(self.index.get(identifier.borrow())?)
    }

    pub fn register(&mut self, resource: Resource) -> Identifier {
        let identifier = resource.identifier.clone();
        self.index.insert(resource.identifier.clone(), resource);

        identifier
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    pub fn values(&self) -> impl Iterator<Item = &Resource> {
        self.index.values()
    }
}