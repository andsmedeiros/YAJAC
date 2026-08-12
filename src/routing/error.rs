use crate::{
    database::error::Error as DatabaseError,
    error::{Source, pointer},
    http_wrappers::StatusCode,
    serialisation::error::Error as SerialisationError,
};
use http::Error as HttpError;
use serde_json::{Error as JsonError, Value, error::Category as JsonCategory, json};
use std::{
    error::Error as StdError,
    fmt::{Display, Formatter},
};

/// A fault raised while serving a request: everything the routing layer itself can refuse, plus the
/// failures of the layers it drives, carried whole.
///
/// Each variant captures what the raising site knew, so the boundary can render a complete error
/// object without any site assembling one by hand. A variant names a `source` only when it can do so
/// truthfully — the standard requires a pointer to address a value that exists in the request
/// document, so a refusal of an absent or unreadable body names nothing.
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// A route template captured no value for a parameter the handler requires.
    RequiredRouteParameterMissing {
        parameter: String,
    },
    FailedToParseRouteParameter {
        parameter: String,
        message: String,
    },

    MissingResourceBody,
    MissingLinkageBody,
    /// The body parsed, but its primary data is not a single resource object.
    PrimaryDataIsNotAResource,
    /// The body is an errors document, which carries no primary data at all.
    ErrorDocumentSubmitted,
    MalformedRequestBody {
        line: usize,
        column: usize,
        message: String,
    },
    /// The body is well-formed JSON that does not model a JSON:API document.
    InvalidRequestBodyContent {
        line: usize,
        column: usize,
        message: String,
    },
    /// The body stream was taken twice — an internal invariant, never a client fault.
    RequestBodyConsumed,
    RequestBodyPeekFailed {
        message: String,
    },

    UnknownAttribute {
        kind: String,
        attribute: String,
    },
    ResourceTypeMismatch {
        expected: String,
        actual: String,
    },
    ResourceIdMismatch {
        expected: String,
        actual: String,
    },
    ResourceIdMissing {
        expected: String,
    },
    ClientGeneratedIdNotSupported {
        kind: String,
    },
    /// Linkage carried a full resource object where a resource identifier object belongs.
    InvalidLinkage,
    /// An identifier names a resource yet to be created (`lid`), which resolves to nothing.
    UnresolvableIdentifier,
    IdentifierTypeMismatch {
        expected: String,
        actual: String,
    },
    InvalidIntegerIdentifier {
        id: String,
    },

    InvalidHeaderValue {
        header: String,
        message: String,
    },
    MissingContentType,
    UnsupportedContentType,
    InvalidContentType,
    ContentTypeCarriesQuality,
    /// The submitted document applies JSON:API extensions the server cannot read.
    UnsupportedJsonApiExtension {
        extensions: Vec<String>,
    },
    InvalidAcceptHeader,
    NoAcceptableMediaType,
    /// Every JSON:API media type offered carries a parameter that rules it out.
    UnusableAcceptMediaTypes,
    /// A response is only acceptable under JSON:API extensions the server cannot produce.
    UnsatisfiableJsonApiExtension {
        extensions: Vec<String>,
    },
    UnsupportedMediaTypeParameter {
        parameter: String,
    },
    UnsupportedQualityValue,
    /// A media type reached JSON:API parsing without being a JSON:API type — callers pre-filter, so
    /// this is a broken invariant rather than a client fault.
    NotAJsonApiMediaType {
        media_type: String,
    },

    ResponseConstructionFailed {
        message: String,
    },
    GeneratedInvalidHeader {
        header: String,
        message: String,
    },

    /// The mount serves no handler for this operation.
    UnsupportedOperation,
    /// A schema-less middleware appears inside the schema-bound chain. The builder rejects this at
    /// assembly; the guard here catches an ordering the types cannot enforce.
    MisorderedMiddleware,

    Database(DatabaseError),
    Serialisation(SerialisationError),
}

impl Error {
    /// Maps the error to the HTTP status its cause mandates: `4xx` for faults the client can
    /// correct, `5xx` for broken server-side invariants.
    pub fn status(&self) -> StatusCode {
        use Error::*;

        match self {
            RequiredRouteParameterMissing { .. }
            | FailedToParseRouteParameter { .. }
            | MalformedRequestBody { .. }
            | RequestBodyPeekFailed { .. }
            | InvalidHeaderValue { .. } => StatusCode::BAD_REQUEST,

            MissingResourceBody
            | MissingLinkageBody
            | PrimaryDataIsNotAResource
            | ErrorDocumentSubmitted
            | InvalidRequestBodyContent { .. }
            | UnknownAttribute { .. }
            | InvalidLinkage
            | UnresolvableIdentifier
            | IdentifierTypeMismatch { .. }
            | InvalidIntegerIdentifier { .. } => StatusCode::UNPROCESSABLE_ENTITY,

            ResourceTypeMismatch { .. } | ResourceIdMismatch { .. } | ResourceIdMissing { .. } => {
                StatusCode::CONFLICT
            }

            ClientGeneratedIdNotSupported { .. } | UnsupportedOperation => StatusCode::FORBIDDEN,

            MissingContentType
            | UnsupportedContentType
            | InvalidContentType
            | ContentTypeCarriesQuality
            | UnsupportedJsonApiExtension { .. }
            | UnsupportedMediaTypeParameter { .. }
            | UnsupportedQualityValue => StatusCode::UNSUPPORTED_MEDIA_TYPE,

            InvalidAcceptHeader
            | NoAcceptableMediaType
            | UnusableAcceptMediaTypes
            | UnsatisfiableJsonApiExtension { .. } => StatusCode::NOT_ACCEPTABLE,

            RequestBodyConsumed
            | NotAJsonApiMediaType { .. }
            | ResponseConstructionFailed { .. }
            | GeneratedInvalidHeader { .. }
            | MisorderedMiddleware => StatusCode::INTERNAL_SERVER_ERROR,

            Database(error) => error.status(),
            Serialisation(error) => error.status(),
        }
    }

    /// Stable, machine-readable code identifying the failure, surfaced as the JSON:API error `code`.
    pub fn code(&self) -> &'static str {
        use Error::*;

        match self {
            RequiredRouteParameterMissing { .. } => "RequiredRouteParameterMissing",
            FailedToParseRouteParameter { .. } => "FailedToParseRouteParameter",
            MissingResourceBody => "MissingResourceBody",
            MissingLinkageBody => "MissingLinkageBody",
            PrimaryDataIsNotAResource => "PrimaryDataIsNotAResource",
            ErrorDocumentSubmitted => "ErrorDocumentSubmitted",
            MalformedRequestBody { .. } => "MalformedRequestBody",
            InvalidRequestBodyContent { .. } => "InvalidRequestBodyContent",
            RequestBodyConsumed => "RequestBodyConsumed",
            RequestBodyPeekFailed { .. } => "RequestBodyPeekFailed",
            UnknownAttribute { .. } => "UnknownAttribute",
            ResourceTypeMismatch { .. } => "ResourceTypeMismatch",
            ResourceIdMismatch { .. } => "ResourceIdMismatch",
            ResourceIdMissing { .. } => "ResourceIdMissing",
            ClientGeneratedIdNotSupported { .. } => "ClientGeneratedIdNotSupported",
            InvalidLinkage => "InvalidLinkage",
            UnresolvableIdentifier => "UnresolvableIdentifier",
            IdentifierTypeMismatch { .. } => "IdentifierTypeMismatch",
            InvalidIntegerIdentifier { .. } => "InvalidIntegerIdentifier",
            InvalidHeaderValue { .. } => "InvalidHeaderValue",
            MissingContentType => "MissingContentType",
            UnsupportedContentType => "UnsupportedContentType",
            InvalidContentType => "InvalidContentType",
            ContentTypeCarriesQuality => "ContentTypeCarriesQuality",
            UnsupportedJsonApiExtension { .. } => "UnsupportedJsonApiExtension",
            InvalidAcceptHeader => "InvalidAcceptHeader",
            NoAcceptableMediaType => "NoAcceptableMediaType",
            UnusableAcceptMediaTypes => "UnusableAcceptMediaTypes",
            UnsatisfiableJsonApiExtension { .. } => "UnsatisfiableJsonApiExtension",
            UnsupportedMediaTypeParameter { .. } => "UnsupportedMediaTypeParameter",
            UnsupportedQualityValue => "UnsupportedQualityValue",
            NotAJsonApiMediaType { .. } => "NotAJsonApiMediaType",
            ResponseConstructionFailed { .. } => "ResponseConstructionFailed",
            GeneratedInvalidHeader { .. } => "GeneratedInvalidHeader",
            UnsupportedOperation => "UnsupportedOperation",
            MisorderedMiddleware => "MisorderedMiddleware",
            Database(error) => error.code(),
            Serialisation(error) => error.code(),
        }
    }

    /// Stable, human-readable summary of the failure, surfaced as the JSON:API error `title`. Unlike
    /// `Display`, it carries no per-occurrence detail.
    pub fn title(&self) -> &'static str {
        use Error::*;

        match self {
            RequiredRouteParameterMissing { .. } => "A required route parameter is missing",
            FailedToParseRouteParameter { .. } => "A route parameter could not be parsed",
            MissingResourceBody => "This request requires a body carrying a resource object",
            MissingLinkageBody => "This request requires a body carrying relationship linkage",
            PrimaryDataIsNotAResource => "The request document has an unexpected shape",
            ErrorDocumentSubmitted => "The request document carries no primary data",
            MalformedRequestBody { .. } => "The request body is not valid JSON",
            InvalidRequestBodyContent { .. } => "The request body is not a valid JSON:API document",
            RequestBodyConsumed => "The request body has already been consumed",
            RequestBodyPeekFailed { .. } => "The request body could not be read",
            UnknownAttribute { .. } => "The resource has no such attribute",
            ResourceTypeMismatch { .. } => "The resource type does not match this endpoint",
            ResourceIdMismatch { .. } => "The resource id does not match this endpoint",
            ResourceIdMissing { .. } => "The submitted resource is missing its id",
            ClientGeneratedIdNotSupported { .. } => {
                "This resource does not accept a client-generated id"
            }
            InvalidLinkage => "Relationship linkage must carry resource identifier objects",
            UnresolvableIdentifier => "This identifier does not reference an existing resource",
            IdentifierTypeMismatch { .. } => "This identifier references the wrong resource type",
            InvalidIntegerIdentifier { .. } => "The identifier is not a valid integer",
            InvalidHeaderValue { .. } => "A request header could not be read",
            MissingContentType => "A 'Content-Type' header is required",
            UnsupportedContentType => "This endpoint does not accept the provided 'Content-Type'",
            InvalidContentType => "The 'Content-Type' header is invalid",
            ContentTypeCarriesQuality => "The 'Content-Type' header carries a quality value",
            UnsupportedJsonApiExtension { .. } => {
                "The request applies a JSON:API extension the server does not support"
            }
            InvalidAcceptHeader => "The 'Accept' header is invalid",
            NoAcceptableMediaType => {
                "This endpoint cannot produce a response in any accepted media type"
            }
            UnusableAcceptMediaTypes => {
                "Every JSON:API media type accepted carries an invalid parameter"
            }
            UnsatisfiableJsonApiExtension { .. } => {
                "A response is only acceptable under a JSON:API extension the server does not support"
            }
            UnsupportedMediaTypeParameter { .. } => "A media type parameter is not supported",
            UnsupportedQualityValue => "A media type quality value is not supported",
            NotAJsonApiMediaType { .. } => "The media type is not a JSON:API media type",
            ResponseConstructionFailed { .. } => "The response could not be constructed",
            GeneratedInvalidHeader { .. } => "The server generated an invalid header",
            UnsupportedOperation => "This endpoint does not support the requested operation",
            MisorderedMiddleware => "The middleware chain is misordered",
            Database(error) => error.title(),
            Serialisation(error) => error.title(),
        }
    }

    /// Names what in the request caused the failure, when the raising site can do so truthfully. A
    /// pointer is offered only where the addressed value is known to exist in the request document.
    pub fn source(&self) -> Option<Source> {
        use Error::*;

        match self {
            PrimaryDataIsNotAResource
            | InvalidLinkage
            | UnresolvableIdentifier
            | IdentifierTypeMismatch { .. }
            | InvalidIntegerIdentifier { .. } => Some(pointer::for_primary_data()),
            UnknownAttribute { attribute, .. } => Some(pointer::for_attribute(attribute)),
            ResourceTypeMismatch { .. } => Some(pointer::for_member("type")),
            ResourceIdMismatch { .. }
            | ResourceIdMissing { .. }
            | ClientGeneratedIdNotSupported { .. } => Some(pointer::for_member("id")),
            InvalidHeaderValue { header, .. } | GeneratedInvalidHeader { header, .. } => {
                Some(Source::Header(header.clone()))
            }
            MissingContentType
            | UnsupportedContentType
            | InvalidContentType
            | ContentTypeCarriesQuality
            | UnsupportedJsonApiExtension { .. }
            | UnsupportedMediaTypeParameter { .. }
            | UnsupportedQualityValue => Some(Source::Header("Content-Type".to_string())),
            InvalidAcceptHeader
            | NoAcceptableMediaType
            | UnusableAcceptMediaTypes
            | UnsatisfiableJsonApiExtension { .. } => Some(Source::Header("Accept".to_string())),

            RequiredRouteParameterMissing { .. }
            | FailedToParseRouteParameter { .. }
            | MissingResourceBody
            | MissingLinkageBody
            | ErrorDocumentSubmitted
            | MalformedRequestBody { .. }
            | InvalidRequestBodyContent { .. }
            | RequestBodyConsumed
            | RequestBodyPeekFailed { .. }
            | NotAJsonApiMediaType { .. }
            | ResponseConstructionFailed { .. }
            | UnsupportedOperation
            | MisorderedMiddleware
            | Database(_)
            | Serialisation(_) => None,
        }
    }

    /// Non-standard detail worth carrying to the client: where in the body a parse failed.
    pub fn meta(&self) -> Option<Value> {
        use Error::*;

        match self {
            MalformedRequestBody { line, column, .. }
            | InvalidRequestBodyContent { line, column, .. } => {
                Some(json!({ "line": line, "column": column }))
            }
            _ => None,
        }
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        use Error::*;

        match self {
            RequiredRouteParameterMissing { parameter } => {
                write!(f, "The route parameter '{parameter}' was not provided")
            }
            FailedToParseRouteParameter { parameter, message } => write!(
                f,
                "Failed to parse the route parameter '{parameter}': {message}"
            ),
            MissingResourceBody => write!(
                f,
                "This request requires a body containing a resource object"
            ),
            MissingLinkageBody => write!(
                f,
                "This request requires a body containing relationship linkage"
            ),
            PrimaryDataIsNotAResource => write!(
                f,
                "The request body must contain a single resource object as its primary data"
            ),
            ErrorDocumentSubmitted => write!(
                f,
                "The request body is an errors document, which carries no primary data"
            ),
            MalformedRequestBody {
                line,
                column,
                message,
            } => write!(
                f,
                "The request body is not valid JSON at line {line}, column {column}: {message}"
            ),
            InvalidRequestBodyContent {
                line,
                column,
                message,
            } => write!(
                f,
                "The request body is not a valid JSON:API document at line {line}, column {column}: {message}"
            ),
            RequestBodyConsumed => write!(f, "The request body has already been consumed"),
            RequestBodyPeekFailed { message } => {
                write!(f, "Failed to read the request body: {message}")
            }
            UnknownAttribute { kind, attribute } => write!(
                f,
                "The resource type '{kind}' has no attribute named '{attribute}'"
            ),
            ResourceTypeMismatch { expected, actual } => write!(
                f,
                "The resource type '{actual}' does not match the '{expected}' resource served at this endpoint"
            ),
            ResourceIdMismatch { expected, actual } => write!(
                f,
                "The resource id '{actual}' does not match the id '{expected}' targeted by this endpoint"
            ),
            ResourceIdMissing { expected } => write!(
                f,
                "The submitted resource must carry the id '{expected}' targeted by this endpoint"
            ),
            ClientGeneratedIdNotSupported { kind } => write!(
                f,
                "The resource type '{kind}' does not accept a client-generated id"
            ),
            InvalidLinkage => write!(
                f,
                "Relationship linkage must contain resource identifier objects, not full resources"
            ),
            UnresolvableIdentifier => write!(
                f,
                "This identifier must reference an existing resource by its id"
            ),
            IdentifierTypeMismatch { expected, actual } => write!(
                f,
                "This identifier references a resource of type '{actual}', but '{expected}' was expected"
            ),
            InvalidIntegerIdentifier { id } => {
                write!(f, "The id '{id}' is not a valid integer identifier")
            }
            InvalidHeaderValue { header, message } => write!(
                f,
                "The '{header}' header contains invalid characters and could not be parsed: {message}"
            ),
            MissingContentType => write!(f, "A 'Content-Type' header must be present and valid"),
            UnsupportedContentType => write!(
                f,
                "This endpoint does not accept the provided 'Content-Type' header"
            ),
            InvalidContentType => write!(
                f,
                "The 'Content-Type' header provided contains an invalid value"
            ),
            ContentTypeCarriesQuality => write!(
                f,
                "The 'Content-Type' header contains an unsupported quality value"
            ),
            UnsupportedJsonApiExtension { extensions } => write!(
                f,
                "The 'Content-Type' header applies JSON:API extensions the server does not support: {}",
                extensions.join(", ")
            ),
            InvalidAcceptHeader => write!(
                f,
                "The 'Accept' header provided contains an invalid value that cannot be parsed"
            ),
            NoAcceptableMediaType => write!(
                f,
                "This endpoint cannot produce a response matching the provided 'Accept' header"
            ),
            UnusableAcceptMediaTypes => write!(
                f,
                "Every JSON:API media type accepted contains an invalid parameter"
            ),
            UnsatisfiableJsonApiExtension { extensions } => write!(
                f,
                "Every JSON:API media type accepted requires an extension the server does not support: {}",
                extensions.join(", ")
            ),
            UnsupportedMediaTypeParameter { parameter } => {
                write!(f, "The media type parameter '{parameter}' is not supported")
            }
            UnsupportedQualityValue => write!(
                f,
                "A media type parameter contains a quality value in an unsupported format"
            ),
            NotAJsonApiMediaType { media_type } => write!(
                f,
                "Attempted to build a JSON:API media type from a header of type '{media_type}'"
            ),
            ResponseConstructionFailed { message } => {
                write!(f, "Failed to construct the response: {message}")
            }
            GeneratedInvalidHeader { header, message } => write!(
                f,
                "The server generated an invalid '{header}' header: {message}"
            ),
            UnsupportedOperation => {
                write!(f, "This endpoint does not support the requested operation")
            }
            MisorderedMiddleware => write!(
                f,
                "A schema-less middleware appears within the schema-bound middleware chain"
            ),
            Database(error) => error.fmt(f),
            Serialisation(error) => error.fmt(f),
        }
    }
}

impl StdError for Error {}

impl From<DatabaseError> for Error {
    fn from(error: DatabaseError) -> Self {
        Error::Database(error)
    }
}

impl From<SerialisationError> for Error {
    fn from(error: SerialisationError) -> Self {
        Error::Serialisation(error)
    }
}

impl From<HttpError> for Error {
    fn from(error: HttpError) -> Self {
        Error::ResponseConstructionFailed {
            message: error.to_string(),
        }
    }
}

impl From<JsonError> for Error {
    /// Splits a body parse by what failed: bytes that are not JSON are a bad request, while JSON
    /// that does not model a document is unprocessable content.
    fn from(error: JsonError) -> Self {
        let (line, column, message) = (error.line(), error.column(), error.to_string());

        match error.classify() {
            JsonCategory::Data => Error::InvalidRequestBodyContent {
                line,
                column,
                message,
            },
            JsonCategory::Io | JsonCategory::Syntax | JsonCategory::Eof => {
                Error::MalformedRequestBody {
                    line,
                    column,
                    message,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_type_mismatch_points_at_the_submitted_type() {
        let error = Error::ResourceTypeMismatch {
            expected: "articles".to_string(),
            actual: "comments".to_string(),
        };

        assert_eq!(error.status(), StatusCode::CONFLICT);
        assert_eq!(error.source(), Some(pointer::for_member("type")));
    }

    #[test]
    fn an_unknown_attribute_points_at_the_attribute() {
        let error = Error::UnknownAttribute {
            kind: "articles".to_string(),
            attribute: "subtitle".to_string(),
        };

        assert_eq!(error.source(), Some(pointer::for_attribute("subtitle")));
    }

    #[test]
    fn a_negotiation_failure_names_the_header_it_read() {
        assert_eq!(
            Error::MissingContentType.source(),
            Some(Source::Header("Content-Type".to_string()))
        );
        assert_eq!(
            Error::NoAcceptableMediaType.source(),
            Some(Source::Header("Accept".to_string()))
        );
    }

    /// A pointer must address a value present in the request document, so a refusal of an absent or
    /// unreadable body names nothing.
    #[test]
    fn a_body_that_never_arrived_names_no_source() {
        assert_eq!(Error::MissingResourceBody.source(), None);
        assert_eq!(Error::ErrorDocumentSubmitted.source(), None);
        assert_eq!(
            Error::MalformedRequestBody {
                line: 1,
                column: 4,
                message: "expected value".to_string(),
            }
            .source(),
            None
        );
    }

    #[test]
    fn a_title_carries_no_occurrence_detail() {
        let one = Error::ResourceIdMismatch {
            expected: "1".to_string(),
            actual: "2".to_string(),
        };
        let another = Error::ResourceIdMismatch {
            expected: "7".to_string(),
            actual: "9".to_string(),
        };

        assert_eq!(one.title(), another.title());
        assert_ne!(one.to_string(), another.to_string());
    }

    #[test]
    fn a_body_parse_failure_carries_its_position() {
        let error = Error::InvalidRequestBodyContent {
            line: 3,
            column: 17,
            message: "invalid type".to_string(),
        };

        assert_eq!(error.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(error.meta(), Some(json!({ "line": 3, "column": 17 })));
    }

    #[test]
    fn a_refusal_with_nothing_to_add_carries_no_meta() {
        assert_eq!(Error::InvalidLinkage.meta(), None);
    }

    #[test]
    fn malformed_json_is_a_bad_request_and_invalid_content_is_unprocessable() {
        let malformed = Error::from(
            serde_json::from_str::<Value>("{").expect_err("unterminated json should not parse"),
        );
        let invalid = Error::from(
            serde_json::from_str::<u32>("\"seven\"").expect_err("a string is not a number"),
        );

        assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);
        assert_eq!(malformed.code(), "MalformedRequestBody");
        assert_eq!(invalid.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(invalid.code(), "InvalidRequestBodyContent");
    }

    /// A funnelled layer is carried whole: every accessor answers as the nested error would.
    #[test]
    fn a_nested_database_failure_answers_for_itself() {
        let nested = DatabaseError::RecordNotFound;
        let error = Error::from(nested.clone());

        assert_eq!(error.status(), nested.status());
        assert_eq!(error.code(), nested.code());
        assert_eq!(error.title(), nested.title());
        assert_eq!(error.to_string(), nested.to_string());
        assert_eq!(error.source(), None);
    }

    #[test]
    fn a_nested_serialisation_failure_answers_for_itself() {
        let nested = SerialisationError::LinkGenerationError {
            message: "the ':tenant' segment was not resolved".to_string(),
        };
        let error = Error::from(nested.clone());

        assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(error.code(), nested.code());
        assert_eq!(error.to_string(), nested.to_string());
    }

    #[test]
    fn a_response_construction_failure_is_internal() {
        let error = Error::ResponseConstructionFailed {
            message: "invalid header value".to_string(),
        };

        assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(error.source(), None);
    }
}
