//! Integration: assert the exact request shape (`?ls=j` + `PW:` header) and the
//! HTTP-status → CoreError mapping, using wiremock.

use dashdown_core::copyparty_client::{CopypartyClient, Credentials};
use dashdown_core::error::CoreError;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn sends_ls_j_query_and_pw_header() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/realdata/"))
        .and(query_param("ls", "j"))
        .and(header("PW", "secret"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"dirs":[],"files":[]}"#))
        .expect(1)
        .mount(&server)
        .await;

    let client =
        CopypartyClient::new(&server.uri(), Credentials::Password("secret".into())).unwrap();
    let listing = client.list_dir("realdata/").await.unwrap();
    assert!(listing.dirs.is_empty() && listing.files.is_empty());
    // `.expect(1)` is verified on drop: confirms the request matched all matchers.
}

#[tokio::test]
async fn maps_401_to_auth_required() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let client = CopypartyClient::new(&server.uri(), Credentials::Anonymous).unwrap();
    let err = client.list_dir("x/").await.unwrap_err();
    assert!(matches!(err, CoreError::AuthRequired), "got {err:?}");
}

#[tokio::test]
async fn maps_403_to_forbidden() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;
    let client = CopypartyClient::new(&server.uri(), Credentials::Password("x".into())).unwrap();
    let err = client.list_dir("x/").await.unwrap_err();
    assert!(matches!(err, CoreError::Forbidden), "got {err:?}");
}

#[tokio::test]
async fn maps_404_to_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let client = CopypartyClient::new(&server.uri(), Credentials::Anonymous).unwrap();
    let err = client.list_dir("missing/").await.unwrap_err();
    assert!(matches!(err, CoreError::NotFound(_)), "got {err:?}");
}
