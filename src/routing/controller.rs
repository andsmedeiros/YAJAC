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
        let parameters = context.query_parameters(schema)?;
        let Composite { content, included } =
            context.store()?.fetch_collection(schema, parameters)?;
        let document = to_document(
            &content,
            included,
            context.uri,
            &DefaultUriGenerator::default(),
        )?;

        respond(Some(document))
    }

    fn show<'req>(context: Context<'sch, 'req, Adapter>) -> Result {
        let schema = Self::resource_schema();
        let parameters = context.query_parameters(schema)?;
        let id = Self::require_id(&context)?;
        let Composite { content, included } =
            context.store()?.fetch_record(schema, id, parameters)?;
        let document = to_document(
            &content,
            included,
            context.uri,
            &DefaultUriGenerator::default(),
        )?;

        respond(Some(document))
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
        let id = Self::require_id(&context)?;
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
        let id = Self::require_id(&context)?;
        context
            .store()?
            .delete_record(Self::resource_schema(), id)?;

        no_content()
    }
}
