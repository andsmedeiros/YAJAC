use crate::json_api::document::Document;
use crate::routing::Error;
use http::{Response, StatusCode};

pub fn respond_with<T>(code: impl Into<StatusCode>, payload: T) -> Result<Response<T>, Error> {
    Ok(Response::builder().status(code.into()).body(payload)?)
}

pub fn respond<T>(payload: T) -> Result<Response<T>, Error> {
    respond_with(StatusCode::OK, payload)
}

pub fn no_content() -> Result<Response<Option<Document>>, Error> {
    respond_with(StatusCode::NO_CONTENT, None)
}
