use crate::{
    adapter::{Adapter, Parameters, UriGenerator},
    resourceful::{
        Resourceful,
        related_data::RelatedData
    },
};
use std::borrow::Borrow;

pub struct Context<'a, G>
where
    G: UriGenerator + 'a
{
    adapter: &'a mut Adapter<G>,
    pub params: Parameters,
}

impl<'a, G> Context<'a, G>
where
    G: UriGenerator + 'a
{
    pub(crate) fn new(adapter: &'a mut Adapter<G>, params: Parameters) -> Self {
        Self { adapter, params }
    }

    pub fn fields_for(&self, kind: impl Borrow<str>) -> Option<&Vec<String>> {
        self.params.fields_for(kind)
    }

    pub fn link_one<N, R>(&mut self, relationship: N, resourceful: Option<R>)
        -> (String, RelatedData)
    where
        N: Borrow<str>,
        R: Resourceful
    {
        let related = match resourceful {
            None => RelatedData::None,
            Some(ref model) => if self.is_included(relationship.borrow()) {
                self.adapter.make_resource(model, &self.params).into()
            } else {
                model.identifier().into()
            }
        };

        (relationship.borrow().into(), related)
    }

    pub fn link_many<N, R, C>(&mut self, relationship: N, collection: C) -> (String, RelatedData)
    where
        N: Borrow<str>,
        R: Resourceful,
        C: IntoIterator<Item = R>
    {
        let related: RelatedData = if self.is_included(relationship.borrow()) {
            collection.into_iter()
                .map(|model| self.adapter.make_resource(&model, &self.params))
                .collect::<Vec<_>>()
                .into()
        } else {
            collection.into_iter()
                .map(|model| model.identifier())
                .collect::<Vec<_>>()
                .into()
        };

        (relationship.borrow().into(), related)
    }

    fn is_included(&self, relationship: impl Borrow<str>) -> bool {
        if let Some(ref includes) = self.params.include {
            includes.iter().any(|r| r == relationship.borrow())
        } else {
            true
        }
    }
}