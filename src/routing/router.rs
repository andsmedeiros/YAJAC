use super::{
    Context, DefaultUriGenerator, Error, Request, Result, RouteParameters,
    controller::{ReadOnlyResourceController, ResourceController},
    default_response, respond_with,
};
use crate::{
    core::factories::to_document,
    database::{adapters::Adapter as AdapterInterface, registry::Registry},
    http_wrappers::{StatusCode, Uri},
    json_api::{document::Document, error::Error as JsonApiError},
};
use http::{Method, Response};
use log::{debug, error};
use std::mem;

pub trait Handler<'sch, Adapter: AdapterInterface>:
    for<'req> Fn(Context<'sch, 'req, Adapter>) -> Result + Sync + Send + 'sch
{
}

impl<'sch, T, Adapter: AdapterInterface> Handler<'sch, Adapter> for T where
    T: for<'req> Fn(Context<'sch, 'req, Adapter>) -> Result + Sync + Send + 'sch
{
}

struct Route<'sch, Adapter: AdapterInterface> {
    method: Method,
    path: Vec<String>,
    handler: Box<dyn Handler<'sch, Adapter>>,
}

impl<'sch, Adapter: AdapterInterface + 'sch> Route<'sch, Adapter> {
    fn new(method: Method, path: Vec<String>, handler: impl Handler<'sch, Adapter>) -> Self {
        Route {
            method,
            path,
            handler: Box::new(handler),
        }
    }

    fn matches(&self, method: &Method, path_segments: &[&str]) -> Option<RouteParameters> {
        if self.method != method || self.path.len() != path_segments.len() {
            return None;
        }

        let mut params = RouteParameters::new();
        for (segment, &path_segment) in self.path.iter().zip(path_segments) {
            if let Some(param_name) = segment.strip_prefix(':') {
                params.insert(param_name, path_segment);
            } else if segment != path_segment {
                return None;
            }
        }
        Some(params)
    }
}

pub struct Router<'sch, Adapter: AdapterInterface> {
    routes: Vec<Route<'sch, Adapter>>,
}

impl<'sch, Adapter: AdapterInterface> Router<'sch, Adapter> {
    pub fn handle(
        &self,
        database: &'sch Registry<'sch, Adapter>,
        request: http::Request<Vec<u8>>,
    ) -> Response<Option<Document>> {
        let uri: Uri = request.uri().clone().into();
        let method = request.method().clone();
        let path_segments: Vec<&str> = uri.path().split('/').filter(|s| !s.is_empty()).collect();

        self.routes
            .iter()
            .find_map(|route| {
                debug!("Matching against {} {}", route.method, route.path.join(""));
                route
                    .matches(&method, &path_segments)
                    .map(|parameters| (route, parameters))
            })
            .map(|(route, parameters)| {
                debug!("Matched!");
                let (parts, body) = request.into_parts();
                let request = Request::from_parts(parts, serde_json::from_slice(&body)?);
                let context = Context::from_request(database, &uri, parameters, request);
                (route.handler)(context)
            })
            .unwrap_or_else(|| {
                Err(Error::new(
                    StatusCode::NOT_FOUND,
                    "ResourceNotFound",
                    format!("{method} {uri}: Resource not found"),
                ))
            })
            .or_else(|error| {
                let status = error.status_code();
                let document = to_document(
                    vec![JsonApiError::from(error)],
                    Vec::new(),
                    &uri,
                    &DefaultUriGenerator::default(),
                )?;

                respond_with(status.into(), Some(document))
            })
            .unwrap_or_else(|error| {
                error!("Failed to construct error response: {error:?}");
                default_response()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(None)
                    .unwrap_or_else(|_| Response::new(None))
            })
    }
}

pub struct RouterBuilder<'sch, Adapter: AdapterInterface> {
    prefix: Vec<String>,
    routes: Vec<Route<'sch, Adapter>>,
}

impl<'sch, Adapter: AdapterInterface> RouterBuilder<'sch, Adapter> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn build(&mut self) -> Router<'sch, Adapter> {
        Router {
            routes: mem::take(&mut self.routes),
        }
    }

    pub fn scope<F>(&mut self, path_segment: &str, f: F) -> &mut Self
    where
        F: FnOnce(&mut Self),
    {
        let mut sub_builder = RouterBuilder {
            prefix: self.prefix.clone(),
            routes: Vec::new(),
        };

        if path_segment != "/" {
            sub_builder.prefix.push(path_segment.to_string());
        }

        f(&mut sub_builder);
        self.routes.extend(sub_builder.routes);
        self
    }

    pub fn get(&mut self, path_segment: &str, handler: impl Handler<'sch, Adapter>) -> &mut Self {
        self.add_route(Method::GET, path_segment, handler)
    }

    pub fn post(&mut self, path_segment: &str, handler: impl Handler<'sch, Adapter>) -> &mut Self {
        self.add_route(Method::POST, path_segment, handler)
    }

    pub fn put(&mut self, path_segment: &str, handler: impl Handler<'sch, Adapter>) -> &mut Self {
        self.add_route(Method::PUT, path_segment, handler)
    }

    pub fn patch(&mut self, path_segment: &str, handler: impl Handler<'sch, Adapter>) -> &mut Self {
        self.add_route(Method::PATCH, path_segment, handler)
    }

    pub fn delete(
        &mut self,
        path_segment: &str,
        handler: impl Handler<'sch, Adapter>,
    ) -> &mut Self {
        self.add_route(Method::DELETE, path_segment, handler)
    }

    fn add_route(
        &mut self,
        method: Method,
        path_segments: &str,
        handler: impl Handler<'sch, Adapter>,
    ) -> &mut Self {
        let route_path = [
            self.prefix.clone(),
            path_segments
                .split("/")
                .filter(|&s| !s.is_empty() && s != "/")
                .map(String::from)
                .collect(),
        ]
        .concat();

        self.routes.push(Route::new(method, route_path, handler));
        self
    }

    pub fn read_only_resource<T>(&mut self, scope: &str) -> &mut Self
    where
        T: ReadOnlyResourceController<'sch, Adapter> + 'sch,
    {
        self.scope(scope, |route| {
            route.get("/", T::index).get("/:id", T::show);
        })
    }

    pub fn resource<T>(&mut self, scope: &str) -> &mut Self
    where
        T: ResourceController<'sch, Adapter> + 'sch,
    {
        self.scope(scope, |route| {
            route
                .get("/", T::index)
                .get("/:id", T::show)
                .post("/", T::create)
                .put("/:id", T::update)
                .patch("/:id", T::update)
                .delete("/:id", T::delete);
        })
    }
}

impl<'sch, Adapter: AdapterInterface> Default for RouterBuilder<'sch, Adapter> {
    fn default() -> Self {
        Self {
            prefix: Vec::new(),
            routes: Vec::new(),
        }
    }
}
