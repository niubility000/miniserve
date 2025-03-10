use crate::{renderer::render_error, MiniserveConfig};
use actix_web::{
    body::AnyBody,
    dev::{ResponseHead, Service, ServiceRequest, ServiceResponse},
    http::{header, StatusCode},
    HttpRequest, HttpResponse, ResponseError,
};
use futures::prelude::*;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ContextualError {
    /// Any kind of IO errors
    #[error("{0}\ncaused by: {1}")]
    IoError(String, std::io::Error),

    /// Might occur during file upload, when processing the multipart request fails
    #[error("Failed to process multipart request\ncaused by: {0}")]
    MultipartError(actix_multipart::MultipartError),

    /// Might occur during file upload
    #[error("File already exists, and the overwrite_files option has not been set")]
    DuplicateFileError,

    /// Any error related to an invalid path (failed to retrieve entry name, unexpected entry type, etc)
    #[error("Invalid path\ncaused by: {0}")]
    InvalidPathError(String),

    /// Might occur if the HTTP credential string does not respect the expected format
    #[error("Invalid format for credentials string. Expected username:password, username:sha256:hash or username:sha512:hash")]
    InvalidAuthFormat,

    /// Might occure if the hash method is neither sha256 nor sha512
    #[error("{0} is not a valid hashing method. Expected sha256 or sha512")]
    InvalidHashMethod(String),

    /// Might occur if the HTTP auth hash password is not a valid hex code
    #[error("Invalid format for password hash. Expected hex code")]
    InvalidPasswordHash,

    /// Might occur if the HTTP auth password exceeds 255 characters
    #[error("HTTP password length exceeds 255 characters")]
    PasswordTooLongError,

    /// Might occur if the user has unsufficient permissions to create an entry in a given directory
    #[error("Insufficient permissions to create file in {0}")]
    InsufficientPermissionsError(String),

    /// Any error related to parsing
    #[error("Failed to parse {0}\ncaused by: {1}")]
    ParseError(String, String),

    /// Might occur when the creation of an archive fails
    #[error("An error occured while creating the {0}\ncaused by: {1}")]
    ArchiveCreationError(String, Box<ContextualError>),

    /// More specific archive creation failure reason
    #[error("{0}")]
    ArchiveCreationDetailError(String),

    /// Might occur when the HTTP credentials are not provided
    #[error("Access requires HTTP authentication")]
    RequireHttpCredentials,

    /// Might occur when the HTTP credentials are not correct
    #[error("Invalid credentials for HTTP authentication")]
    InvalidHttpCredentials,

    /// Might occur when an HTTP request is invalid
    #[error("Invalid HTTP request\ncaused by: {0}")]
    InvalidHttpRequestError(String),

    /// Might occur when trying to access a page that does not exist
    #[error("Route {0} could not be found")]
    RouteNotFoundError(String),

    /// In case miniserve was invoked without an interactive terminal and without an explicit path
    #[error("Refusing to start as no explicit serve path was set and no interactive terminal was attached
Please set an explicit serve path like: `miniserve /my/path`")]
    NoExplicitPathAndNoTerminal,

    /// In case miniserve was invoked with --no-symlinks but the serve path is a symlink
    #[error("The -P|--no-symlinks option was provided but the serve path '{0}' is a symlink")]
    NoSymlinksOptionWithSymlinkServePath(String),
}

impl ResponseError for ContextualError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::ArchiveCreationError(_, err) => err.status_code(),
            Self::RouteNotFoundError(_) => StatusCode::NOT_FOUND,
            Self::InsufficientPermissionsError(_) => StatusCode::FORBIDDEN,
            Self::InvalidHttpCredentials | Self::RequireHttpCredentials => StatusCode::UNAUTHORIZED,
            Self::InvalidHttpRequestError(_) => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_response(&self) -> HttpResponse {
        if let Self::RequireHttpCredentials = self {
        } else {
            log_error_chain(self.to_string());
        }

        let mut resp = HttpResponse::build(self.status_code());
        if let Self::RequireHttpCredentials | Self::InvalidHttpCredentials = self {
            resp.append_header((
                header::WWW_AUTHENTICATE,
                header::HeaderValue::from_static("Basic realm=\"miniserve\""),
            ));
        }

        resp.content_type("text/plain; charset=utf-8")
            .body(self.to_string())
    }
}

/// Middleware to convert plain-text error responses to user-friendly web pages
pub fn error_page_middleware<S>(
    req: ServiceRequest,
    srv: &S,
) -> impl Future<Output = actix_web::Result<ServiceResponse>> + 'static
where
    S: Service<ServiceRequest, Response = ServiceResponse, Error = actix_web::Error>,
    S::Future: 'static,
{
    let fut = srv.call(req);

    async {
        let res = fut.await?;

        if (res.status().is_client_error() || res.status().is_server_error())
            && res.headers().get(header::CONTENT_TYPE).map(AsRef::as_ref)
                == Some(b"text/plain; charset=utf-8")
        {
            let req = res.request().clone();
            Ok(res.map_body(|head, body| map_error_page(&req, head, body)))
        } else {
            Ok(res)
        }
    }
}

fn map_error_page(req: &HttpRequest, head: &mut ResponseHead, body: AnyBody) -> AnyBody {
    let error_msg = match &body {
        AnyBody::Bytes(bytes) => match std::str::from_utf8(bytes) {
            Ok(msg) => msg,
            _ => return body,
        },
        _ => return body,
    };

    let conf = req.app_data::<MiniserveConfig>().unwrap();
    let return_address = req
        .headers()
        .get(header::REFERER)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("/");

    head.headers.insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("text/html; charset=utf-8"),
    );

    render_error(error_msg, head.status, conf, return_address)
        .into_string()
        .into()
}

pub fn log_error_chain(description: String) {
    for cause in description.lines() {
        log::error!("{}", cause);
    }
}
