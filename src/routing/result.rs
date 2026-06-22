use super::error::Error;
use crate::json_api::document::Document;
use http::Response;
use std::result::Result as StdResult;

pub type Result = StdResult<Response<Option<Document>>, Error>;
