use super::{
    Context, Request, Result, RouteParameters,
    controller::{ReadOnlyResourceController, ResourceController},
    default_response,
};
use crate::database::{adapters::Adapter as AdapterInterface, registry::Registry};
use http::{Method, Response, StatusCode};
use log::debug;
use serde_json::{Value, json};
use std::mem;

pub trait Handler<'a, Adapter: AdapterInterface + 'a>:
    Fn(Request, Context<'a, Adapter>) -> Result + Sync + Send + 'a
{
}

impl<'a, T: 'a, Adapter: AdapterInterface + 'a> Handler<'a, Adapter> for T where
    T: Fn(Request, Context<Adapter>) -> Result + Sync + Send
{
}

struct Route<'a, Adapter: AdapterInterface + 'a> {
    method: Method,
    path: Vec<String>,
    handler: Box<dyn Handler<'a, Adapter>>,
}

impl<'a, Adapter: AdapterInterface + 'a> Route<'a, Adapter> {
    fn new(method: Method, path: Vec<String>, handler: impl Handler<'a, Adapter>) -> Self {
        Route {
            method,
            path,
            handler: Box::new(handler),
        }
    }

    fn matches(&self, method: &Method, path_segments: &[&str]) -> Option<RouteParameters> {
        if &self.method != method || self.path.len() != path_segments.len() {
            return None;
        }

        let mut params = RouteParameters::new();
        for (segment, path_segment) in self.path.iter().zip(path_segments) {
            if segment.starts_with(':') {
                let param_name = &segment[1..];
                params.insert(param_name.to_string(), path_segment.clone());
            } else if segment != path_segment {
                return None;
            }
        }
        Some(params)
    }
}

pub struct Router<'a, Adapter: AdapterInterface> {
    routes: Vec<Route<'a, Adapter>>,
}

impl<'a, Adapter: AdapterInterface + 'a> Router<'a, Adapter> {
    pub fn handle(&self, database: &'a Registry<'a, Adapter>, request: Request) -> Response<Value> {
        let uri = request.uri().clone();
        let path_segments: Vec<&str> = uri
            .path()
            .split('/')
            .filter(|s| !s.is_empty())
            .collect();

        for route in &self.routes {
            debug!("Matching against {} {}", route.method, route.path.join(""));
            if let Some(parameters) = route.matches(request.method(), &path_segments) {
                debug!("Matched!");

                return Context::try_new(database, uri.into(), parameters)
                    .and_then(|context| (route.handler)(request, context))
                    .unwrap_or_else(|error| {
                        default_response()
                            .status(error.status_code())
                            .body(serde_json::to_value(&error).unwrap())
                            .expect("Failed to construct error response")
                    });
            }
        }

        default_response()
            .status(StatusCode::NOT_FOUND)
            .body(json!(format!(
                "{} {}: Resource not found",
                request.method(),
                request.uri().path()
            )))
            .expect("Failed to construct not found response")
    }
}

pub struct RouterBuilder<'a, Adapter: AdapterInterface> {
    prefix: Vec<String>,
    routes: Vec<Route<'a, Adapter>>,
}

impl<'a, Adapter: AdapterInterface + 'a> RouterBuilder<'a, Adapter> {
    pub fn new() -> Self {
        RouterBuilder {
            prefix: Vec::new(),
            routes: Vec::new(),
        }
    }

    pub fn build(&mut self) -> Router<'a, Adapter> {
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

    pub fn get(&mut self, path_segment: &str, handler: impl Handler<'a, Adapter>) -> &mut Self {
        self.add_route(Method::GET, path_segment, handler)
    }

    pub fn post(&mut self, path_segment: &str, handler: impl Handler<'a, Adapter>) -> &mut Self {
        self.add_route(Method::POST, path_segment, handler)
    }

    pub fn put(&mut self, path_segment: &str, handler: impl Handler<'a, Adapter>) -> &mut Self {
        self.add_route(Method::PUT, path_segment, handler)
    }

    pub fn patch(&mut self, path_segment: &str, handler: impl Handler<'a, Adapter>) -> &mut Self {
        self.add_route(Method::PATCH, path_segment, handler)
    }

    pub fn delete(&mut self, path_segment: &str, handler: impl Handler<'a, Adapter>) -> &mut Self {
        self.add_route(Method::DELETE, path_segment, handler)
    }

    fn add_route(
        &mut self,
        method: Method,
        path_segments: &str,
        handler: impl Handler<'a, Adapter>,
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
        T: ReadOnlyResourceController<'a, Adapter> + 'a,
    {
        self.scope(scope, |route| {
            route.get("/", T::index).get("/:id", T::show);
        })
    }

    pub fn resource<T>(&mut self, scope: &str) -> &mut Self
    where
        T: ResourceController<'a, Adapter> + 'a,
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
