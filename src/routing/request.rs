use http::Request as HttpRequest;
use serde_json::Value;

pub type Request = HttpRequest<Value>;
