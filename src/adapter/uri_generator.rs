use crate::{
    http_wrappers::Uri,
    spec::identifier::Identifier
};

const GENERATED_INVALID_MSG: &'static str = "Generated an invalid URI";

pub trait UriGenerator {
    fn base_url(&self) -> String { "".to_string() }

    fn uri_for_resource(&self, identifier: &Identifier) -> Uri {
        let base = self.base_url();

        if let Identifier::Existing { kind, id } = identifier  {
            format!("{base}/{kind}/{id}")
                .parse::<Uri>()
                .expect(GENERATED_INVALID_MSG)
        } else {
            panic!("Attempted to generate URI for unpersisted resource");
        }
    }

    fn uri_for_relationship(&self, identifier: &Identifier, relationship: &str) -> Uri {
        let resource = self.uri_for_resource(identifier);
        format!("{resource}/relationships/{relationship}")
            .parse::<Uri>()
            .expect(GENERATED_INVALID_MSG)
    }

    fn uri_for_related(&self, identifier: &Identifier, relationship: &str) -> Uri {
        let resource = self.uri_for_resource(identifier);
        format!("{resource}/{relationship}")
            .parse::<Uri>()
            .expect(GENERATED_INVALID_MSG)
    }
}

pub struct DefaultUriGenerator<'a> {
    protocol: &'a str,
    host: &'a str,
    namespace: &'a str
}

impl<'a> DefaultUriGenerator<'a> {
    pub fn new(protocol: &'a str, host: &'a str, namespace: &'a str) -> Self {
        assert!(
            !protocol.is_empty() && !host.is_empty() ||
                protocol.is_empty() && host.is_empty(),
            "URL protocol and host must either be both absent of both present."
        );
        DefaultUriGenerator { protocol, host, namespace }
    }
}

impl Default for DefaultUriGenerator<'_> {
    fn default() -> Self {
        DefaultUriGenerator::new("", "", "")
    }
}

impl<'a> UriGenerator for DefaultUriGenerator<'a> {
    fn base_url(&self) -> String {
        if self.protocol.is_empty() && self.host.is_empty() {
            self.namespace.to_string()
        } else {
            format!("{}://{}:{}", self.protocol, self.host, self.namespace)
        }

    }
}
