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

pub trait ResourceController<'sch, Adapter: AdapterInterface> {
    fn resource_schema() -> &'sch TableSchema<'sch>;

    fn index(request: Request, context: Context<Adapter>) -> Result {
        let table = context.database.table(Self::resource_schema().name)?;
        let mut collection = table.query(&context.parameters.query)?;
        let included = DataLoader::new(context.database)
            .load_for_collection(&mut collection, &context.parameters.query)?;
        let document = to_document(
            &collection,
            included,
            context.uri,
            &context.parameters.query,
            &DefaultUriGenerator::default(),
        )?;

        respond(to_value(&document)?)
    }

    fn show(request: Request, context: Context<Adapter>) -> Result;
    fn create(request: Request, context: Context<Adapter>) -> Result;
    fn update(request: Request, context: Context<Adapter>) -> Result;
    fn delete(request: Request, context: Context<Adapter>) -> Result;
}
