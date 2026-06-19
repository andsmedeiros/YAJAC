use super::error::Error;
use crate::{
    database::{
        adapters::Adapter as AdapterInterface, query_parameters::QueryParameters,
        registry::Registry,
    },
    http_wrappers::Uri,
    routing::RouteParameters,
};
use crate::database::schema::TableSchema;

#[derive(Debug, Clone)]
pub struct Parameters<'sch, 'req> {
    pub route: RouteParameters,
    pub query: QueryParameters<'sch, 'req>,
}

pub struct Context<'sch, 'reg, 'req, Adapter: AdapterInterface> {
    pub schema: &'sch TableSchema<'sch>,
    pub registry: &'reg Registry<'sch, Adapter>,
    pub uri: & 'req Uri,
    pub parameters: Parameters<'sch, 'req>,
}

impl<'sch, 'reg, 'req, Adapter: AdapterInterface> Context<'sch, 'reg, 'req, Adapter> {
    pub fn try_new(
        schema: &'sch TableSchema<'sch>,
        registry: &'reg Registry<'sch, Adapter>,
        uri: &'req Uri,
        route_parameters: RouteParameters,
    ) -> Result<Self, Error> {
        let parameters = Parameters {
            query: QueryParameters::parse(uri, schema, registry)?,
            route: route_parameters,
        };
        Ok(Context {
            schema,
            registry,
            uri,
            parameters,
        })
    }
}
