use crate::{
    database::adapters::Adapter as AdapterInterface,
    routing::{Context, Request, Result}
};

pub trait ReadOnlyResourceController<Adapter: AdapterInterface> {
    fn index(request: Request, context: Context<Adapter>) -> Result;
    fn show(request: Request, context: Context<Adapter>) -> Result;
}

pub trait ResourceController<Adapter: AdapterInterface> {
    fn index(request: Request, context: Context<Adapter>) -> Result;
    fn show(request: Request, context: Context<Adapter>) -> Result;
    fn create(request: Request, context: Context<Adapter>) -> Result;
    fn update(request: Request, context: Context<Adapter>) -> Result;
    fn delete(request: Request, context: Context<Adapter>) -> Result;
}