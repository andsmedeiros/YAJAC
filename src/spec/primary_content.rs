use serde::{Deserialize, Serialize};
use crate::spec::{
    error::Error,
    resource::Resource
};

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PrimaryContent {
    Record { data: Resource },
    Collection { data: Vec<Resource> },
    Errors { errors: Vec<Error> },
}

impl From<Resource> for PrimaryContent {
    fn from(resource: Resource) -> Self {
        PrimaryContent::Record { data: resource }
    }
}

impl From<Vec<Resource>> for PrimaryContent {
    fn from(collection: Vec<Resource>) -> Self {
        PrimaryContent::Collection { data: collection }
    }
}

impl<const N: usize> From<[Resource; N]> for PrimaryContent {
    fn from(collection: [Resource; N]) -> Self {
        PrimaryContent::Collection { data: collection.into() }
    }
}

impl From<Vec<Error>> for PrimaryContent {
    fn from(errors: Vec<Error>) -> Self {
        PrimaryContent::Errors { errors }
    }
}

impl<const N: usize> From<[Error; N]> for PrimaryContent {
    fn from(errors: [Error; N]) -> Self {
        PrimaryContent::Errors { errors: errors.into() }
    }
}