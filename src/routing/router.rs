use super::{
    Context, Request, Result, RouteParameters,
    controller::{ReadOnlyResourceController, ResourceController},
    default_response,
};
use crate::{
    database::{adapters::Adapter as AdapterInterface, registry::Registry},
    http_wrappers::{StatusCode, Uri},
};
use http::{Method, Response};
use log::{debug, error};
use serde_json::{Value, json};
use std::mem;

pub trait Handler<'sch, Adapter: AdapterInterface>:
    for<'req> Fn(Request, Context<'sch, 'req, Adapter>) -> Result + Sync + Send + 'sch
{
}

impl<'sch, T, Adapter: AdapterInterface> Handler<'sch, Adapter> for T where
    T: for<'req> Fn(Request, Context<'sch, 'req, Adapter>) -> Result + Sync + Send + 'sch
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
        request: Request,
    ) -> Response<Value> {
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
            .and_then(|(route, parameters)| {
                debug!("Matched!");
                let context = Context::new(database, &uri, parameters);
                (route.handler)(request, context).into()
            })
            .unwrap_or_else(|| {
                default_response()
                    .status(StatusCode::NOT_FOUND)
                    .body(json!({
                        "status": StatusCode::NOT_FOUND,
                        "code": "ResourceNotFound",
                        "title": format!(
                            "{} {}: Resource not found",
                            method,
                            uri
                        )
                    }))
                    .map_err(Into::into)
            })
            .or_else(|error| {
                serde_json::to_value(&error)
                    .map_err(|error| {
                        error!("Failed to serialise error to json: {:?}", error);
                    })
                    .and_then(|value| {
                        default_response()
                            .status(error.status_code())
                            .body(value)
                            .map_err(|error| {
                                error!("Failed to construct error response: {:?}", error);
                            })
                    })
            })
            .or_else(|_| {
                default_response()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(json!({
                        "status": StatusCode::INTERNAL_SERVER_ERROR,
                        "code": "ResponseConstructionError",
                        "title": "The server failed to construct an error response"
                    }))
            })
            .map_err(|error| {
                error!("Failed to construct fallback error response: {:?}", error);
            })
            .expect("Failed to construct response")
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
