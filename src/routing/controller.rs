use crate::routing::{Context, Request, Result};

pub trait ReadOnlyResourceController<Connection> {
    fn index(request: Request, context: Context<Connection>) -> Result;
    fn show(request: Request, context: Context<Connection>) -> Result;
}

pub trait ResourceController<Connection> {
    fn index(request: Request, context: Context<Connection>) -> Result;
    fn show(request: Request, context: Context<Connection>) -> Result;
    fn create(request: Request, context: Context<Connection>) -> Result;
    fn update(request: Request, context: Context<Connection>) -> Result;
    fn delete(request: Request, context: Context<Connection>) -> Result;
}