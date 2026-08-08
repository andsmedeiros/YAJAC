use crate::serialisation::ByteStream;
use http::Request as HttpRequest;

/// The primary-tier inbound request: an HTTP request whose body is a raw byte stream. The resource
/// tier has no request analogue — it is entered through a `ResourceContext` (schema plus the parsed
/// document), not an `http::Request` of documents.
pub type PrimaryRequest = HttpRequest<ByteStream>;
