use super::error::Error;
use crate::database::schema::TableSchema;
use crate::{
    database::{
        adapters::Adapter as AdapterInterface, query_parameters::QueryParameters,
        registry::Registry,
    },
    http_wrappers::Uri,
    routing::RouteParameters,
};

pub struct Parameters<'sch, 'req> {
    route: RouteParameters,
    query: std::cell::OnceCell<QueryParameters<'sch, 'req>>,
}

pub struct Context<'sch, 'req, Adapter: AdapterInterface> {
    pub registry: &'sch Registry<'sch, Adapter>,
    pub uri: &'req Uri,
    pub parameters: Parameters<'sch, 'req>,
}

impl<'sch, 'req, Adapter: AdapterInterface> Context<'sch, 'req, Adapter> {
    pub fn new(
        registry: &'sch Registry<'sch, Adapter>,
        uri: &'req Uri,
        route_parameters: RouteParameters,
    ) -> Self {
        Self {
            registry,
            uri,
            parameters: Parameters {
                route: route_parameters,
                query: Default::default(),
            },
        }
    }

    pub fn route_parameters(&self) -> &RouteParameters {
        &self.parameters.route
    }

    pub fn query_parameters(
        &self,
        schema: &'sch TableSchema<'sch>,
    ) -> Result<&QueryParameters<'sch, 'req>, Error> {
        match self.parameters.query.get() {
            Some(parameters) => Ok(parameters),
            None => {
                let parameters = QueryParameters::parse(self.uri, schema, self.registry)?;
                Ok(self.parameters.query.get_or_init(|| parameters))
            }
        }
    }
}
