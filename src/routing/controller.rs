use crate::{
    core::factories::to_document,
    database::{
        adapters::Adapter as AdapterInterface,
        attributes::Identifier,
        composite::Composite,
        error::Error as DatabaseError,
        query_parameters::QueryParameters,
        record::Record,
        schema::{IdentifierType, TableSchema},
        store::Store,
    },
    http_wrappers::Uri,
    routing::{Context, DefaultUriGenerator, Error, Result, responder::*},
};
use http::StatusCode;

/// A request narrowed to a single resource: the resource's schema paired with the
/// routing context. It lends the context's request operations already bound to that
/// schema, so controller handlers never thread the schema through by hand.
pub struct ResourceContext<'sch, 'req, Adapter: AdapterInterface + 'sch> {
    schema: &'sch TableSchema<'sch>,
    context: Context<'sch, 'req, Adapter>,
}

impl<'sch, 'req, Adapter: AdapterInterface + 'sch> ResourceContext<'sch, 'req, Adapter> {
    pub fn new(schema: &'sch TableSchema<'sch>, context: Context<'sch, 'req, Adapter>) -> Self {
        Self { schema, context }
    }

    pub fn schema(&self) -> &'sch TableSchema<'sch> {
        self.schema
    }

    pub fn uri(&self) -> &'req Uri {
        self.context.uri
    }

    /// Parses the request body into a record validated against the resource schema.
    pub fn require_record(&mut self) -> std::result::Result<Record<'sch>, Error> {
        self.context.require_record(self.schema)
    }

    /// Lazily parses the query string against the resource schema.
    pub fn query_parameters(
        &self,
    ) -> std::result::Result<&QueryParameters<'sch, 'req>, DatabaseError> {
        self.context.query_parameters(self.schema)
    }

    pub fn store(&self) -> std::result::Result<Store<'sch, '_, Adapter>, DatabaseError> {
        self.context.store()
    }

    /// Resolves the endpoint's `:id` route parameter into a typed primary key.
    pub fn require_id(&self) -> std::result::Result<Identifier, Error> {
        let parameters = self.context.route_parameters();
        let identifier = match self.schema.primary_key().kind {
            IdentifierType::Text => Identifier::Text(parameters.require_as("id")?),
            IdentifierType::Integer => Identifier::Integer(parameters.require_as("id")?),
        };

        Ok(identifier)
    }
}

pub trait ReadOnlyResourceController<'sch, Adapter: AdapterInterface + 'sch> {
    fn index<'req>(context: ResourceContext<'sch, 'req, Adapter>) -> Result
    where
        'sch: 'req,
    {
        serve_index(context)
    }

    fn show<'req>(context: ResourceContext<'sch, 'req, Adapter>) -> Result
    where
        'sch: 'req,
    {
        serve_show(context)
    }
}

pub trait ResourceController<'sch, Adapter: AdapterInterface + 'sch> {
    fn index<'req>(context: ResourceContext<'sch, 'req, Adapter>) -> Result
    where
        'sch: 'req,
    {
        serve_index(context)
    }

    fn show<'req>(context: ResourceContext<'sch, 'req, Adapter>) -> Result
    where
        'sch: 'req,
    {
        serve_show(context)
    }

    fn create<'req>(mut context: ResourceContext<'sch, 'req, Adapter>) -> Result
    where
        'sch: 'req,
    {
        let new_record = context.require_record()?;
        let parameters = context.query_parameters()?;
        let Composite { content, included } =
            context.store()?.create_record(new_record, parameters)?;
        let document = to_document(
            &content,
            included,
            context.uri(),
            &DefaultUriGenerator::default(),
        )?;

        respond_with(StatusCode::CREATED, Some(document))
    }

    fn update<'req>(mut context: ResourceContext<'sch, 'req, Adapter>) -> Result
    where
        'sch: 'req,
    {
        let record = context.require_record()?;
        let parameters = context.query_parameters()?;
        let Composite { content, included } = context.store()?.update_record(record, parameters)?;
        let document = to_document(
            &content,
            included,
            context.uri(),
            &DefaultUriGenerator::default(),
        )?;

        respond(Some(document))
    }

    fn delete<'req>(context: ResourceContext<'sch, 'req, Adapter>) -> Result
    where
        'sch: 'req,
    {
        let id = context.require_id()?;
        context.store()?.delete_record(context.schema(), id)?;

        no_content()
    }
}

fn serve_index<'sch, 'req, Adapter: AdapterInterface + 'sch>(
    context: ResourceContext<'sch, 'req, Adapter>,
) -> Result {
    let parameters = context.query_parameters()?;
    let Composite { content, included } = context
        .store()?
        .fetch_collection(context.schema(), parameters)?;
    let document = to_document(
        &content,
        included,
        context.uri(),
        &DefaultUriGenerator::default(),
    )?;

    respond(Some(document))
}

fn serve_show<'sch, 'req, Adapter: AdapterInterface + 'sch>(
    context: ResourceContext<'sch, 'req, Adapter>,
) -> Result {
    let parameters = context.query_parameters()?;
    let id = context.require_id()?;
    let Composite { content, included } =
        context
            .store()?
            .fetch_record(context.schema(), id, parameters)?;
    let document = to_document(
        &content,
        included,
        context.uri(),
        &DefaultUriGenerator::default(),
    )?;

    respond(Some(document))
}
