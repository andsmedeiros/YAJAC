use crate::{
    adapter::{Cache, Parameters, UriGenerator, make_resource},
    resourceful::{
        Resourceful,
        related_data::RelatedData
    },
};
use std::{
    borrow::Borrow,
    cell::RefCell,
    rc::Rc
};

pub struct Context<G: UriGenerator> {
    pub(super) cache: Rc<RefCell<Cache>>,
    pub(super) params: Parameters,
    pub(super) uri_generator: G,
}

impl<'a, G: UriGenerator> Context<G>{
    pub(crate) fn new(cache: Rc<RefCell<Cache>>, params: Parameters, uri_generator: G) -> Self {
        Self { cache, params, uri_generator }
    }

    pub fn fields_for(&self, kind: impl Borrow<str>) -> Option<&Vec<String>> {
        self.params.fields_for(kind)
    }

    pub fn link_one<N, R>(&self, relationship: N, resourceful: Option<R>)
        -> (String, RelatedData)
    where
        N: Borrow<str>,
        R: Resourceful
    {
        let related = match resourceful {
            None => RelatedData::None,
            Some(ref model) => if self.is_included(relationship.borrow()) {
                make_resource(model, self).into()
            } else {
                model.identifier().into()
            }
        };

        (relationship.borrow().into(), related)
    }

    pub fn link_many<N, R, C>(&self, relationship: N, collection: C) -> (String, RelatedData)
    where
        N: Borrow<str>,
        R: Resourceful,
        C: IntoIterator<Item = R>
    {
        let related: RelatedData = if self.is_included(relationship.borrow()) {
            collection.into_iter()
                .map(|model| make_resource(&model, self))
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