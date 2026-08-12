use crate::error::Error;
use crate::json_api::document::Document;
use crate::serialisation::ByteStream;
use http::Response;
use std::error::Error as StdError;
use std::result::Result as StdResult;

/// The resource-tier handler result: a document response, or the crate error the crossing renders
/// into a JSON:API error document.
pub type ResourceResult = StdResult<Response<Option<Document>>, Error>;

/// The primary-tier handler result: a streamed byte response, fallible with a boxed error. The byte
/// tier's error is not JSON:API-shaped, so it escapes `Router::handle` to the embedder rather than
/// being rendered.
pub type PrimaryResult = StdResult<Response<Option<ByteStream>>, Box<dyn StdError>>;
