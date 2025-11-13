use crate::database::QueryParameters;
use crate::database::adapters::Adapter as AdapterInterface;
use crate::database::error::Error;
use crate::database::registry::Registry;
use crate::database::schema::{Relationship, TableSchema};
use crate::database::table::Table;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
pub struct IncludeNode<'sch: 'req, 'req> {
    relationship: &'req str,
    descriptor: &'sch Relationship<'sch>,
    children: BTreeMap<&'sch str, IncludeNode<'sch, 'req>>,
}

impl<'a, 'b> PartialEq for IncludeNode<'a, 'b> {
    fn eq(&self, other: &Self) -> bool {
        self.relationship == other.relationship
    }
}

impl<'a, 'b> Eq for IncludeNode<'a, 'b> {}

impl<'a, 'b> PartialOrd for IncludeNode<'a, 'b> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<'a, 'b> Ord for IncludeNode<'a, 'b> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.relationship.cmp(&other.relationship)
    }
}

pub type RequestedFields<'sch> = BTreeMap<&'sch str, BTreeSet<&'sch str>>;
pub type IncludedModels<'sch, 'req> = BTreeMap<&'sch str, IncludeNode<'sch, 'req>>;
pub type ModelsToLoad<'sch> = BTreeMap<&'sch str, &'sch TableSchema<'sch>>;

pub struct LoadContext<'sch: 'req, 'req, Adapter: AdapterInterface> {
    pub schema: &'sch TableSchema<'sch>,
    pub registry: &'req Registry<'sch, Adapter>,
    pub fields: RequestedFields<'req>,
    pub include: IncludedModels<'sch, 'req>,
}

impl<'sch: 'req, 'req, Adapter: AdapterInterface> LoadContext<'sch, 'req, Adapter> {
    pub fn new(
        schema: &'sch TableSchema<'sch>,
        registry: &'req Registry<'sch, Adapter>,
        query_parameters: &'req QueryParameters,
    ) -> Result<LoadContext<'sch, 'req, Adapter>, Error> {
        let (include, models) = extract_models(schema, registry, query_parameters)?;

        let fields: RequestedFields = if let Some(ref requested_fields) = query_parameters.fields {
            models
                .iter()
                .map(|(model, schema)| {
                    match requested_fields
                        .iter()
                        .find(|(name, _)| name.as_str() == *model)
                    {
                        Some((_, fields)) => {
                            (*model, fields.iter().map(|field| field.as_str()).collect())
                        }
                        None => (*model, schema.fields().collect()),
                    }
                })
                .collect()
        } else {
            models
                .iter()
                .map(|(model, schema)| (*model, schema.fields().collect()))
                .collect()
        };

        Ok(Self {
            schema,
            registry,
            fields,
            include,
        })
    }

    pub fn derive(&self, relationship: &str) -> Result<LoadContext<'sch, 'req, Adapter>, Error> {
        let include =
            self.include
                .get(relationship)
                .ok_or_else(|| Error::SchemaValidationFailure {
                    schema: self.schema.name.to_string(),
                    attribute: relationship.to_string(),
                    message: "Invalid relationship requested".to_string(),
                })?;

        let schema = self
            .registry
            .table(include.descriptor.related_resource().resource)?
            .schema();

        Ok(Self {
            schema,
            registry: self.registry,
            fields: self.fields.clone(),
            include: include.children.clone(),
        })
    }

    pub fn is_requested(&self, field: &str) -> bool {
        match self.fields.get(self.schema.name) {
            Some(fields) => fields.contains(field),
            None => false,
        }
    }

    pub fn is_included(&self, relationship: &str) -> bool {
        self.include.contains_key(relationship)
    }

    pub fn should_load(&self, relationship: &str) -> bool {
        self.is_requested(relationship) || self.is_included(relationship)
    }

    pub fn relationships_to_load(
        &self,
    ) -> impl Iterator<Item = &'sch (&'sch str, Relationship<'sch>)> {
        self.schema
            .relationships
            .iter()
            .filter(|(relationship, _)| self.should_load(*relationship))
    }
}

fn extract_models<'sch: 'req, 'req, Adapter: AdapterInterface>(
    schema: &'sch TableSchema<'sch>,
    registry: &'req Registry<'sch, Adapter>,
    query_parameters: &'req QueryParameters,
) -> Result<(IncludedModels<'sch, 'req>, ModelsToLoad<'sch>), Error> {
    let mut included = IncludedModels::new();
    let mut models = ModelsToLoad::from_iter([(schema.name, schema)]);

    if let Some(ref relationships_paths) = query_parameters.include {
        for relationship_path in relationships_paths {
            let mut relationship;
            let mut rest = Some(relationship_path.as_str());
            let mut scope = &mut included;
            let mut schema = schema;

            while let Some(path) = rest {
                (relationship, rest) = match path.split_once(".") {
                    Some((relationship, rest)) => (relationship, Some(rest)),
                    None => (path, None),
                };

                let (relationship, descriptor) = schema
                    .relationships
                    .iter()
                    .find(|(r, _)| relationship == *r)
                    .ok_or_else(|| Error::SchemaValidationFailure {
                        schema: schema.name.to_string(),
                        attribute: relationship.to_string(),
                        message: "Invalid relationship requested".to_string(),
                    })?;

                schema = registry
                    .table(descriptor.related_resource().resource)?
                    .schema();

                models.insert(schema.name, schema);

                scope = &mut scope
                    .entry(relationship)
                    .or_insert(IncludeNode {
                        relationship,
                        descriptor,
                        children: BTreeMap::new(),
                    })
                    .children;
            }
        }
    }

    Ok((included, models))
}
