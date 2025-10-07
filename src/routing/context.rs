use crate::{
    database::QueryParameters,
    http_wrappers::Uri,
    routing::RouteParameters,
};
use super::error::Error;

#[derive(Debug, Clone)]
pub struct Parameters {
    pub route: RouteParameters,
    pub query: QueryParameters,
}

#[derive(Debug)]
pub struct Context<'a, Connection> {
    pub database: &'a mut Connection,
    pub uri: Uri,
    pub parameters: Parameters,
}

impl<'a, Connection> Context<'a, Connection> {
    pub fn try_new(
        database: &'a mut Connection,
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