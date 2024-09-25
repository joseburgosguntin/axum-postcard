// postcard deps
use postcard::{from_bytes, to_allocvec};
use serde::{de::DeserializeOwned, Serialize};
// axum deps
use async_trait::async_trait;
use axum::{
    body::{Body, Bytes},
    extract::{rejection::BytesRejection, FromRequest},
    http::{header, HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
};

/// Postcard Extractor / Response.
///
/// When used as an extractor, it can deserialize request bodies into some type that
/// implements [`serde::Deserialize`]. The request will be rejected (and a [`PostcardRejection`] will
/// be returned) if:
///
/// - The request doesn't have a `Content-Type: application/postcard` (or similar) header.
/// - The body doesn't contain syntactically valid Postcard.
/// - The body contains syntactically valid Postcard but it couldn't be deserialized into the target
/// type.
/// - Buffering the request body fails.
///
/// ⚠️ Since parsing Postcard requires consuming the request body, the `Postcard` extractor must be
/// *last* if there are multiple extractors in a handler.
/// See ["the order of extractors"][order-of-extractors]
///
/// [order-of-extractors]: crate::extract#the-order-of-extractors
///
/// See [`PostcardRejection`] for more details.
///
/// # Extractor example
///
/// ```rust,no_run
/// use axum::{
///     extract,
///     routing::post,
///     Router,
/// };
/// use serde::Deserialize;
/// use axum_postcard::Postcard;
///
/// #[derive(Deserialize)]
/// struct CreateUser {
///     email: String,
///     password: String,
/// }
///
/// async fn create_user(Postcard(payload): Postcard<CreateUser>) {
///     // payload is a `CreateUser`
///     todo!()
/// }
///
/// let app = Router::new().route("/users", post(create_user));
/// # async {
/// # let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
/// # axum::serve(listener, app).await.unwrap();
/// # };
/// ```
///
/// When used as a response, it can serialize any type that implements [`serde::Serialize`] to
/// `Postcard`, and will automatically set `Content-Type: application/postcard` header.
///
/// # Response example
///
/// ```
/// use axum::{
///     extract::Path,
///     routing::get,
///     Router,
/// };
/// use serde::Serialize;
/// use axum_postcard::Postcard;
///
/// #[derive(Serialize)]
/// struct User {
///     id: u32,
///     username: String,
/// }
///
/// async fn get_user(Path(user_id) : Path<u32>) -> Postcard<User> {
///     let user = find_user(user_id).await;
///     Postcard(user)
/// }
///
/// async fn find_user(user_id: u32) -> User {
///     todo!()
/// }
///
/// let app = Router::new().route("/users/:id", get(get_user));
/// # async {
/// # let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
/// # axum::serve(listener, app).await.unwrap();
/// # };
/// ```
pub struct Postcard<T>(pub T);

#[derive(thiserror::Error, Debug)]
pub enum PostcardRejection {
    #[error("Expected request with `Content-Type: application/postcard`")]
    MissingPostcardContentType,
    #[error(transparent)]
    PostcardError(#[from] postcard::Error),
    #[error(transparent)]
    Bytes(#[from] BytesRejection),
}

impl IntoResponse for PostcardRejection {
    fn into_response(self) -> Response {
        use PostcardRejection::*;
        // its often easiest to implement `IntoResponse` by calling other implementations
        match self {
            MissingPostcardContentType => {
                (StatusCode::UNSUPPORTED_MEDIA_TYPE, self.to_string()).into_response()
            }
            PostcardError(err) => (StatusCode::BAD_REQUEST, err.to_string()).into_response(),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()).into_response(),
        }
    }
}

#[async_trait]
impl<T, S> FromRequest<S> for Postcard<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = PostcardRejection;

    async fn from_request(req: Request<Body>, state: &S) -> Result<Self, Self::Rejection> {
        if postcard_content_type(req.headers()) {
            let bytes = Bytes::from_request(req, state).await?;

            let value = match from_bytes(&*bytes) {
                Ok(value) => value,
                Err(err) => return Err(PostcardRejection::PostcardError(err)),
            };
            Ok(Postcard(value))
        } else {
            Err(PostcardRejection::MissingPostcardContentType)
        }
    }
}

fn postcard_content_type(headers: &HeaderMap) -> bool {
    let content_type = if let Some(content_type) = headers.get(header::CONTENT_TYPE) {
        content_type
    } else {
        return false;
    };

    let content_type = if let Ok(content_type) = content_type.to_str() {
        content_type
    } else {
        return false;
    };

    let mime = if let Ok(mime) = content_type.parse::<mime::Mime>() {
        mime
    } else {
        return false;
    };

    let is_postcard_content_type = mime.type_() == "application"
        && (mime.subtype() == "postcard" || mime.suffix().map_or(false, |name| name == "postcard"));

    is_postcard_content_type
}

impl<T> IntoResponse for Postcard<T>
where
    T: Serialize,
{
    fn into_response(self) -> Response {
        // TODO: maybe use 128 bytes cause serde is doing something like that
        match to_allocvec(&self.0) {
            Ok(value) => ([(header::CONTENT_TYPE, "application/postcard")], value).into_response(),
            Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::post, Router};
    use axum_test_helpers::*;
    use serde::Deserialize;
    use futures_util::StreamExt;

    #[tokio::test]
    async fn deserialize_body() {
        #[derive(Debug, Deserialize, Serialize)]
        struct Input {
            foo: String,
        }

        let app = Router::new().route("/", post(|input: Postcard<Input>| async { input.0.foo }));

        let client = TestClient::new(app);

        let res = client
            .post("/")
            .header("content-type", "application/postcard")
            .body("\x03bar")
            .await;
        let body = res.text().await;

        assert_eq!(body, "bar");
    }

    #[tokio::test]
    async fn consume_body_to_postcard_requires_postcard_content_type() {
        #[derive(Debug, Deserialize)]
        struct Input {
            foo: String,
        }

        let app = Router::new().route("/", post(|input: Postcard<Input>| async { input.0.foo }));

        let client = TestClient::new(app);
        let res = client.post("/").body("\x03bar").await;

        let status = res.status();

        assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[tokio::test]
    async fn postcard_content_types() {
        async fn valid_postcard_content_type(content_type: &str) -> bool {
            println!("testing {content_type:?}");

            let app = Router::new().route("/", post(|Postcard(_): Postcard<String>| async {}));

            let res = TestClient::new(app)
                .post("/")
                .header("content-type", content_type)
                .body("\x02hi")
                .await;

            res.status() == StatusCode::OK
        }

        assert!(valid_postcard_content_type("application/postcard").await);
        assert!(valid_postcard_content_type("application/postcard; charset=utf-8").await);
        assert!(valid_postcard_content_type("application/postcard;charset=utf-8").await);
        assert!(valid_postcard_content_type("application/cloudevents+postcard").await);
        assert!(!valid_postcard_content_type("text/postcard").await);
    }

    #[tokio::test]
    async fn invalid_postcard_syntax() {
        let app = Router::new().route("/", post(|_: Postcard<String>| async {}));

        let client = TestClient::new(app);
        let res = client
            .post("/")
            .body("\x03")
            .header("content-type", "application/postcard")
            .await;

        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[derive(Deserialize)]
    struct Foo {
        #[allow(dead_code)]
        a: i32,
        #[allow(dead_code)]
        b: Vec<Bar>,
    }

    #[derive(Deserialize)]
    struct Bar {
        #[allow(dead_code)]
        x: i32,
        #[allow(dead_code)]
        y: i32,
    }

    #[tokio::test]
    async fn invalid_postcard_data() {
        let app = Router::new().route("/", post(|_: Postcard<Foo>| async {}));

        let client = TestClient::new(app);
        let res = client
            .post("/")
            .header("content-type", "application/postcard")
            .body("\x02\x01\x04")
            .await;

        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let body_text = res.text().await;
        assert_eq!(body_text, "Hit the end of buffer, expected more data");
    }

    #[tokio::test]
    async fn serialize_response() {
        let response = Postcard("bar").into_response();

        assert!(postcard_content_type(response.headers()));

        let mut body = Vec::new();
        let body_parts: Vec<_> = response.into_body().into_data_stream().collect().await;
        for part in body_parts {
            body.extend_from_slice(&part.unwrap());
        }

        assert_eq!(body, b"\x03bar");
    }
}
