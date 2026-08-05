#[cfg(test)]
mod tests;

pub mod base_uri;
pub mod builders;
pub mod context;
pub mod controller;
pub mod error;
pub mod mount_table;
pub mod request;
pub mod responder;
pub mod result;
mod route_parameters;
pub mod router;

pub use base_uri::BaseUri;
pub use builders::{
    PrimaryRouteBuilder, RelationshipConfig, RelationshipsConfig, ResourceHandler,
    ResourceRouteBuilder, RouteBuilder, SubordinateRouteBuilder, UnboundVerbs,
};
pub use context::PrimaryContext;
pub use controller::DefaultController;
pub use error::Error;
pub(crate) use mount_table::MountTable;
pub use request::Request;
pub use responder::{default_response, respond, respond_with};
pub use result::Result;
pub use route_parameters::RouteParameters;
pub use router::{MountSlot, Router, RouterError};
