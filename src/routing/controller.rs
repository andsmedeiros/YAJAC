use crate::{
    core::factories::to_document,
    database::{
        adapters::Adapter as AdapterInterface,
        attributes::{Identifier, from_value},
        data_loader::DataLoader,
        schema::{IdentifierType, TableSchema},
        table::Table,
    },
    routing::{Context, DefaultUriGenerator, Error, Result, responder::*},
};
use http::StatusCode;
use serde_json::to_value;

pub trait ReadOnlyResourceController<'sch, Adapter: AdapterInterface + 'sch> {
    fn resource_schema() -> &'sch TableSchema<'sch>;

    fn index<'req>(context: Context<'sch, 'req, Adapter>) -> Result;
    fn show<'req>(context: Context<'sch, 'req, Adapter>) -> Result;
}

pub trait ResourceController<'sch, Adapter: AdapterInterface + 'sch> {
    fn resource_schema() -> &'sch TableSchema<'sch>;

    fn require_id<'req>(
        context: &Context<'sch, 'req, Adapter>,
    ) -> std::result::Result<Identifier, Error> {
        let parameters = context.route_parameters();
        let identifier = match Self::resource_schema().primary_key.kind {
            IdentifierType::Text => Identifier::Text(parameters.require_as("id")?),
            IdentifierType::Integer => Identifier::Integer(parameters.require_as("id")?),
        };

        Ok(identifier)
    }

    fn index<'req>(context: Context<'sch, 'req, Adapter>) -> Result {
        let schema = Self::resource_schema();
        let query_parameters = context.query_parameters(schema)?;
        let mut collection = context.table(schema.name)?.query(query_parameters)?;
        let included = DataLoader::new(context.registry, context.connection()?)
            .load_for_collection(&mut collection, query_parameters)?;
        let document = to_document(
            &collection,
            included,
            context.uri,
            &DefaultUriGenerator::default(),
        )?;

        respond(to_value(&document)?)
    }

    fn show<'req>(context: Context<'sch, 'req, Adapter>) -> Result {
        let schema = Self::resource_schema();
        let query_parameters = context.query_parameters(schema)?;
        let id = Self::require_id(&context)?;
        let mut record = context.table(schema.name)?.find(id, query_parameters)?;
        let included = DataLoader::new(context.registry, context.connection()?)
            .load_for_record(&mut record, query_parameters)?;
        let document = to_document(
            &record,
            included,
            context.uri,
            &DefaultUriGenerator::default(),
        )?;

        respond(to_value(&document)?)
    }

    fn create<'req>(context: Context<'sch, 'req, Adapter>) -> Result {
        let schema = Self::resource_schema();
        let query_parameters = context.query_parameters(schema)?;
        let attributes = from_value(schema, context.body().clone())?;

        let mut record = context
            .transaction(|cx| cx.table(schema.name)?.insert(attributes, query_parameters))?;
        let included = DataLoader::new(context.registry, context.connection()?)
            .load_for_record(&mut record, query_parameters)?;
        let document = to_document(
            &record,
            included,
            context.uri,
            &DefaultUriGenerator::default(),
        )?;

        respond_with(StatusCode::CREATED, to_value(&document)?)
    }

    fn update<'req>(context: Context<'sch, 'req, Adapter>) -> Result {
        let schema = Self::resource_schema();
        let query_parameters = context.query_parameters(schema)?;
        let id = Self::require_id(&context)?;
        let attributes = from_value(schema, context.body().clone())?;

        let mut record = context.transaction(|cx| {
            cx.table(schema.name)?
                .update(id, attributes, query_parameters)
        })?;
        let included = DataLoader::new(context.registry, context.connection()?)
            .load_for_record(&mut record, query_parameters)?;
        let document = to_document(
            &record,
            included,
            context.uri,
            &DefaultUriGenerator::default(),
        )?;

        respond(to_value(&document)?)
    }

    fn delete<'req>(context: Context<'sch, 'req, Adapter>) -> Result {
        let id = Self::require_id(&context)?;
        context.table(Self::resource_schema().name)?.delete(id)?;

        no_content()
    }
}
