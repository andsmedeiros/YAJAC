use crate::{
    core::factories::to_document,
    database::{
        adapters::Adapter as AdapterInterface,
        attributes::Identifier,
        composite::Composite,
        schema::{IdentifierType, TableSchema},
    },
    routing::{Context, DefaultUriGenerator, Error, Result, responder::*},
};
use http::StatusCode;

pub trait ReadOnlyResourceController<'sch, Adapter: AdapterInterface + 'sch> {
    fn resource_schema() -> &'sch TableSchema<'sch>;

    fn index<'req>(context: Context<'sch, 'req, Adapter>) -> Result {
        serve_index(Self::resource_schema(), context)
    }

    fn show<'req>(context: Context<'sch, 'req, Adapter>) -> Result {
        serve_show(Self::resource_schema(), context)
    }
}

pub trait ResourceController<'sch, Adapter: AdapterInterface + 'sch> {
    fn resource_schema() -> &'sch TableSchema<'sch>;

    fn index<'req>(context: Context<'sch, 'req, Adapter>) -> Result {
        serve_index(Self::resource_schema(), context)
    }

    fn show<'req>(context: Context<'sch, 'req, Adapter>) -> Result {
        serve_show(Self::resource_schema(), context)
    }

    fn create<'req>(context: Context<'sch, 'req, Adapter>) -> Result {
        let schema = Self::resource_schema();
        let parameters = context.query_parameters(schema)?;
        let store = context.store()?;

        let new_record = store.materialise_new(context.require_resource(schema)?, schema)?;
        let Composite { content, included } = store.create_record(new_record, parameters)?;
        let document = to_document(
            &content,
            included,
            context.uri,
            &DefaultUriGenerator::default(),
        )?;

        respond_with(StatusCode::CREATED, Some(document))
    }

    fn update<'req>(context: Context<'sch, 'req, Adapter>) -> Result {
        let schema = Self::resource_schema();
        let parameters = context.query_parameters(schema)?;
        let id = require_id(schema, &context)?;
        let store = context.store()?;

        let record = store.materialise(context.require_resource(schema)?, schema, id)?;
        let Composite { content, included } = store.update_record(record, parameters)?;
        let document = to_document(
            &content,
            included,
            context.uri,
            &DefaultUriGenerator::default(),
        )?;

        respond(Some(document))
    }

    fn delete<'req>(context: Context<'sch, 'req, Adapter>) -> Result {
        let schema = Self::resource_schema();
        let id = require_id(schema, &context)?;
        context.store()?.delete_record(schema, id)?;

        no_content()
    }
}

fn serve_index<'sch, 'req, Adapter: AdapterInterface + 'sch>(
    schema: &'sch TableSchema<'sch>,
    context: Context<'sch, 'req, Adapter>,
) -> Result {
    let parameters = context.query_parameters(schema)?;
    let Composite { content, included } = context.store()?.fetch_collection(schema, parameters)?;
    let document = to_document(
        &content,
        included,
        context.uri,
        &DefaultUriGenerator::default(),
    )?;

    respond(Some(document))
}

fn serve_show<'sch, 'req, Adapter: AdapterInterface + 'sch>(
    schema: &'sch TableSchema<'sch>,
    context: Context<'sch, 'req, Adapter>,
) -> Result {
    let parameters = context.query_parameters(schema)?;
    let id = require_id(schema, &context)?;
    let Composite { content, included } = context.store()?.fetch_record(schema, id, parameters)?;
    let document = to_document(
        &content,
        included,
        context.uri,
        &DefaultUriGenerator::default(),
    )?;

    respond(Some(document))
}

fn require_id<'sch, 'req, Adapter: AdapterInterface + 'sch>(
    schema: &'sch TableSchema<'sch>,
    context: &Context<'sch, 'req, Adapter>,
) -> std::result::Result<Identifier, Error> {
    let parameters = context.route_parameters();
    let identifier = match schema.primary_key.kind {
        IdentifierType::Text => Identifier::Text(parameters.require_as("id")?),
        IdentifierType::Integer => Identifier::Integer(parameters.require_as("id")?),
    };

    Ok(identifier)
}
