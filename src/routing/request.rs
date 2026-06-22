use crate::json_api::document::Document;
use http::Request as HttpRequest;

pub type Request = HttpRequest<Option<Document>>;
