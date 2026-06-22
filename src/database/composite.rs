use crate::database::record::Record;

pub struct Composite<'sch, T> {
    pub content: T,
    pub included: Vec<Record<'sch>>,
}

pub type CompositeRecord<'sch> = Composite<'sch, Record<'sch>>;
pub type CompositeCollection<'sch> = Composite<'sch, Vec<Record<'sch>>>;
