use serde_json::Value;
use http::Request as HttpRequest;

pub type Request = HttpRequest<Value>;