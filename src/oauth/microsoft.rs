use oauth2::url::Url;
use oauth2::{basic::BasicClient, TokenResponse};
// Alternatively, this can be oauth2::curl::http_client or a custom.
use oauth2::reqwest::async_http_client;
use oauth2::{
    AuthType, AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, RedirectUrl, TokenUrl,
};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

use webbrowser;

use super::OauthClient;

pub struct MicrosoftOauthClient {
    inner: BasicClient,
}

const AUTH_URL: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/authorize";
const TOKEN_URL: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/token";
const REDIRECT_URL: &str = "http://localhost:3003/redirect";

impl MicrosoftOauthClient {
    pub fn new(client_id: &str, client_secret: &str) -> Self {
        let auth_url =
            AuthUrl::new(AUTH_URL.to_string()).expect("Invalid authorization endpoint URL");
        let token_url = TokenUrl::new(TOKEN_URL.to_string()).expect("Invalid token endpoint URL");

        let client = BasicClient::new(
            ClientId::new(client_id.to_string()),
            Some(ClientSecret::new(client_secret.to_string())),
            auth_url,
            Some(token_url),
        )
        .set_auth_type(AuthType::RequestBody)
        .set_redirect_uri(
            RedirectUrl::new(REDIRECT_URL.to_string()).expect("Invalid redirect URL"),
        );

        Self { inner: client }
    }

    pub async fn refresh_access_token(&self, refresh_token: String) -> (String, String) {
        let token = self
            .inner
            .exchange_refresh_token(&oauth2::RefreshToken::new(refresh_token))
            .request_async(async_http_client)
            .await;

        let inner = token.unwrap();

        (
            inner.access_token().secret().to_owned(),
            inner.refresh_token().unwrap().secret().to_owned(),
        )
    }

    pub async fn get_authorization_code(&self) -> (String, String) {
        let (authorize_url, _csrf_state, pkce_code_verifier) =
            self.inner.get_authorization_url(vec![
                "https://graph.microsoft.com/Calendars.ReadWrite",
                "https://graph.microsoft.com/User.Read",
                "offline_access",
            ]); // "https://graph.microsoft.com/Calendars.Write", "https://graph.microsoft.com/User.Read"

        webbrowser::open(authorize_url.as_str()).expect("failed to open web browser");

        let mut token = None;

        // A very naive implementation of the redirect server.
        let listener = TcpListener::bind("127.0.0.1:3003").unwrap();
        for stream in listener.incoming() {
            if let Ok(mut stream) = stream {
                let code;
                let _state;
                {
                    let mut reader = BufReader::new(&stream);

                    let mut request_line = String::new();
                    reader.read_line(&mut request_line).unwrap();

                    let redirect_url = request_line.split_whitespace().nth(1).unwrap();
                    let url = Url::parse(&("http://localhost".to_string() + redirect_url)).unwrap();

                    let code_pair = url.query_pairs().find(|pair| {
                        let &(ref key, _) = pair;
                        key == "code"
                    });

                    if code_pair.is_none() {
                        break;
                    }

                    let (_, value) = code_pair.unwrap();
                    code = AuthorizationCode::new(value.into_owned());

                    let state_pair = url
                        .query_pairs()
                        .find(|pair| {
                            let &(ref key, _) = pair;
                            key == "state"
                        })
                        .unwrap();

                    let (_, value) = state_pair;
                    _state = CsrfToken::new(value.into_owned());
                }

                let message = "Go back to your terminal :)";
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n{}",
                    message.len(),
                    message
                );
                stream.write_all(response.as_bytes()).unwrap();

                // Exchange the code with a token.
                let token_result = self
                    .inner
                    .exchange_code(code)
                    // Send the PKCE code verifier in the token request
                    .set_pkce_verifier(pkce_code_verifier)
                    .request_async(async_http_client)
                    .await;

                token = Some(token_result.unwrap());

                // The server will terminate itself after collecting the first code.
                break;
            }
        }

        let inner = token.unwrap();
        let access_token = inner.access_token().secret().to_owned();
        let refresh_token = inner.refresh_token().unwrap().secret().to_owned();
        (access_token, refresh_token)
    }
}
