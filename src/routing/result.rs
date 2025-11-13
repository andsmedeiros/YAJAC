use super::error::Error;
use http::Response;
use serde_json::Value;
use std::result::Result as StdResult;

pub type Result = StdResult<Response<Value>, Error>;
