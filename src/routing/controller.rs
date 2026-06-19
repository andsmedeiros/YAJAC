use crate::{
    core::factories::to_document,
    database::{
        adapters::Adapter as AdapterInterface, data_loader::DataLoader, schema::TableSchema,
        table::Table,
    },
    routing::{Context, DefaultUriGenerator, Request, Result, responder::*},
};

use serde_json::to_value;

pub trait ReadOnlyResourceController<'sch, Adapter: AdapterInterface> {
    fn resource_schema() -> &'sch TableSchema<'sch>;

    fn index(request: Request, context: Context<Adapter>) -> Result;
    fn show(request: Request, context: Context<Adapter>) -> Result;
}

pub trait ResourceController<'sch, Adapter: AdapterInterface + 'sch> {
    fn resource_schema() -> &'sch TableSchema<'sch>;

    fn index<'req>(_: Request, context: Context<'sch, 'req, Adapter>) -> Result {
        let table = context.registry.table(Self::resource_schema().name)?;
        let query_parameters = context.query_parameters(Self::resource_schema())?;
        let mut collection = table.query(query_parameters)?;
        let included = DataLoader::new(context.registry)
            .load_for_collection(&mut collection, query_parameters)?;
        let document = to_document(
            &collection,
            included,
            context.uri,
            &DefaultUriGenerator::default(),
        )?;

        respond(to_value(&document)?)
    }

    fn show(request: Request, context: Context<Adapter>) -> Result;
    fn create(request: Request, context: Context<Adapter>) -> Result;
    fn update(request: Request, context: Context<Adapter>) -> Result;
    fn delete(request: Request, context: Context<Adapter>) -> Result;
}
