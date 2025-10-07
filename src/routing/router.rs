use std::mem;
use log::debug;
use serde_json::{json, Value};
use http::{Method, Response, StatusCode};
use crate::{
    routing::controller::{ReadOnlyResourceController, ResourceController},
    routing::{default_response, Context, Request, Result},
};
use crate::routing::parameters::RouteParameters;

pub trait Handler<'a, Connection: 'a>: Fn(Request, Context<'a, Connection>) -> Result + Sync + Send + 'a {  }

impl<'a, T: 'a, Connection: 'a> Handler<'a, Connection> for T
where
    T: Fn(Request, Context<Connection>) -> Result + Sync + Send { }

struct Route<'a, Connection> {
    method: Method,
    path: Vec<String>,
    handler: Box<dyn Handler<'a, Connection>>,
}

impl<'a, Connection: 'a> Route<'a, Connection> {
    fn new(method: Method, path: Vec<String>, handler: impl Handler<'a, Connection>) -> Self {
        Route {
            method,
            path,
            handler: Box::new(handler),
        }
    }

    fn matches(&self, method: &Method, path_segments: &[String])
        -> Option<RouteParameters>
    {
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

pub struct Router<'a, Connection> {
    routes: Vec<Route<'a, Connection>>
}

impl<'a, Connection: 'a> Router<'a, Connection> {
    pub fn handle(&self, database: &'a mut Connection, request: Request) -> Response<Value> {
        let uri = request.uri().clone();
        let path_segments: Vec<String> = uri
            .path()
            .split('/')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();

        for route in &self.routes {
            debug!("Matching against {} {}", route.method, route.path.join(""));
            if let Some(parameters) = route.matches(request.method(), &path_segments) {
                debug!("Matched!");

                return Context::try_new(database, uri.into(), parameters)
                    .and_then(|context|
                        (route.handler)(request, context)
                    )
                    .unwrap_or_else(|error|
                        default_response()
                            .status(error.status_code())
                            .body(serde_json::to_value(&error).unwrap())
                            .expect("Failed to construct error response")
                    );
            }
        }

        default_response()
            .status(StatusCode::NOT_FOUND)
            .body(json!(format!("{} {}: Resource not found", request.method(), request.uri().path())))
            .expect("Failed to construct not found response")
    }
}

pub struct RouterBuilder<'a, Connection> {
    prefix: Vec<String>,
    routes: Vec<Route<'a, Connection>>,
}

impl<'a, Connection: 'a> RouterBuilder<'a, Connection> {
    pub fn new() -> Self {
        RouterBuilder {
            prefix: Vec::new(),
            routes: Vec::new(),
        }
    }

    pub fn build(&mut self) -> Router<'a, Connection> {
        Router { routes: mem::take(&mut self.routes) }
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

    pub fn get(&mut self, path_segment: &str, handler: impl Handler<'a, Connection>) -> &mut Self {
        self.add_route(Method::GET, path_segment, handler)
    }

    pub fn post(&mut self, path_segment: &str, handler: impl Handler<'a, Connection>) -> &mut Self {
        self.add_route(Method::POST, path_segment, handler)
    }

    pub fn put(&mut self, path_segment: &str, handler: impl Handler<'a, Connection>) -> &mut Self {
        self.add_route(Method::PUT, path_segment, handler)
    }

    pub fn patch(&mut self, path_segment: &str, handler: impl Handler<'a, Connection>) -> &mut Self {
        self.add_route(Method::PATCH, path_segment, handler)
    }

    pub fn delete(&mut self, path_segment: &str, handler: impl Handler<'a, Connection>) -> &mut Self {
        self.add_route(Method::DELETE, path_segment, handler)
    }

    fn add_route(&mut self, method: Method, path_segments: &str, handler: impl Handler<'a, Connection>) -> &mut Self {
        let route_path = [
            self.prefix.clone(),
            path_segments
                .split("/")
                .filter(|&s| !s.is_empty() && s != "/")
                .map(String::from)
                .collect()
        ].concat();

        self.routes.push(Route::new(method, route_path, handler));
        self
    }

    pub fn read_only_resource<T>(&mut self, scope: &str) -> &mut Self
    where
        T: ReadOnlyResourceController<Connection> + 'a
    {
        self.scope(scope, |route| {
            route
                .get("/", T::index)
                .get("/:id", T::show);
        })
    }

    pub fn resource<T>(&mut self, scope: &str) -> &mut Self
    where
        T: ResourceController<Connection> + 'a
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