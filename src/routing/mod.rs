#[cfg(test)]
mod tests;

pub mod builders;
pub mod context;
pub mod controller;
pub mod error;
pub mod request;
pub mod responder;
pub mod result;
mod route_parameters;
pub mod router;
pub mod uri_generator;

pub use builders::{
    PrimaryRouteBuilder, RelationshipConfig, RelationshipsConfig, ResourceHandler,
    ResourceRouteBuilder, RouteBuilder, SubordinateRouteBuilder, UnboundVerbs,
};
pub use context::Context;
pub use controller::DefaultController;
pub use error::Error;
pub use request::Request;
pub use responder::{default_response, respond, respond_with};
pub use result::Result;
pub use route_parameters::RouteParameters;
pub use router::{MountSlot, MountTable, Router, RouterError};
pub use uri_generator::{DefaultUriGenerator, UriGenerator};
