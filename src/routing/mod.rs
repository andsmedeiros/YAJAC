#[cfg(test)]
mod tests;

pub mod base_uri;
pub mod builders;
pub mod context;
pub mod controller;
pub mod error;
pub mod middleware;
pub mod mount_table;
pub mod request;
pub mod responder;
pub mod result;
mod route_parameters;
pub mod router;

pub use base_uri::BaseUri;
pub use builders::{
    PrimaryRouteBuilder, RelationshipConfig, RelationshipsConfig, ResourceRouteBuilder,
    ResourceVerbs, RouteBuilder, SubordinateRouteBuilder, UnboundVerbs,
};
pub use context::{PrimaryContext, ResourceContext};
pub use error::Error;
pub use middleware::{PrimaryHandler, PrimaryMiddleware, ResourceHandler, ResourceMiddleware};
pub(crate) use mount_table::MountTable;
pub use request::PrimaryRequest;
pub use responder::{respond, respond_with};
pub use result::{PrimaryResult, ResourceResult};
pub use route_parameters::RouteParameters;
pub use router::{MountSlot, Router, RouterError};
