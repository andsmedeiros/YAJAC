use crate::routing::Error;
use http::{Response, StatusCode, response::Builder as ResponseBuilder};
use serde_json::Value;
pub fn default_response() -> ResponseBuilder {
    Response::builder()
        .header("Access-Control-Allow-Origin", "*")
        .header("Content-Type", "application/json; charset=utf-8")
}

pub fn respond_with<T>(code: StatusCode, payload: T) -> Result<Response<T>, Error> {
    Ok(default_response().status(code).body(payload)?)
}

pub fn respond<T>(payload: T) -> Result<Response<T>, Error> {
    respond_with(StatusCode::OK, payload)
}

pub fn no_content() -> Result<Response<Value>, Error> {
    respond_with(StatusCode::NO_CONTENT, Value::Null)
}
