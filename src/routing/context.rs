use crate::{
    database::{
        adapters::Adapter as AdapterInterface,
        query_parameters::QueryParameters,
        registry::Registry,
    },
    http_wrappers::Uri,
    routing::RouteParameters,
};
use super::error::Error;

#[derive(Debug, Clone)]
pub struct Parameters {
    pub route: RouteParameters,
    pub query: QueryParameters,
}

pub struct Context<'a, Adapter: AdapterInterface> {
    pub database: &'a Registry<'a, Adapter>,
    pub uri: Uri,
    pub parameters: Parameters,
}

impl<'a, Adapter: AdapterInterface> Context<'a, Adapter> {
    pub fn try_new(
        database: &'a Registry<'a, Adapter>,
        uri: Uri,
        route_parameters: RouteParameters,
    )
        -> Result<Self, Error>
    {
        let parameters = Parameters {
            query: QueryParameters::parse(&uri)?,
            route: route_parameters
        };
        Ok(Context { database, uri, parameters })
    }
}