use crate::database::{
    adapters::Adapter as AdapterInterface,
    attributes::Attributes,
    registry::Registry
};
use std::collections::HashMap;

type RecordCache = HashMap<&'static str, HashMap<i32, Attributes>>;

pub fn load_for_collection<Adapter: AdapterInterface>(registry: Registry<Adapter>, &[&Attributes]) -> RecordCache {

}

pub fn load_for_record<Adapter: AdapterInterface>(registry: Registry<Adapter>, record: &Attributes) -> RecordCache {
    load_for_collection(registry, &[record])
}