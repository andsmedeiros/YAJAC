use serde_json::Value;
use std::result::Result as StdResult;
use http::Response;
use super::error::Error;

pub type Result = StdResult<Response<Value>, Error>;