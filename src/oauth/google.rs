use oauth2::url::Url;
use oauth2::{basic::BasicClient, TokenResponse};
// Alternatively, this can be oauth2::curl::http_client or a custom.
use oauth2::reqwest::async_http_client;
use oauth2::{
    AuthType, AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, PkceCodeChallenge,
    RedirectUrl, Scope, TokenUrl,
};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

use webbrowser;

use super::OauthClient;

pub struct GoogleOauthClient {
    inner: BasicClient,
}

impl GoogleOauthClient {
    pub fn new(client_id: &str, client_secret: &str, _auth_url: &str, _token_url: &str) -> Self {
        let auth_url = AuthUrl::new("https://accounts.google.com/o/oauth2/v2/auth".to_string())
            .expect("Invalid authorization endpoint URL");
        let token_url = TokenUrl::new("https://www.googleapis.com/oauth2/v3/token".to_string())
            .expect("Invalid token endpoint URL");

        let client = BasicClient::new(
            ClientId::new(client_id.to_string()),
            Some(ClientSecret::new(client_secret.to_string())),
            auth_url,
            Some(token_url),
        )
        .set_auth_type(AuthType::RequestBody)
        .set_redirect_uri(
            RedirectUrl::new("http://localhost:3003/redirect".to_string())
                .expect("Invalid redirect URL"),
        );

        Self { inner: client }
    }

    pub async fn refresh_access_token(&self, refresh_token: String) -> String {
        let token = self
            .inner
            .exchange_refresh_token(&oauth2::RefreshToken::new(refresh_token))
            .request_async(async_http_client)
            .await;

        let inner = token.unwrap();

        inner.access_token().secret().to_owned()
    }

    pub async fn get_authorization_code(&self) -> (String, String) {
        let (authorize_url, csrf_state, pkce_code_verifier) =
            self.inner.get_authorization_url(vec![
                "https://www.googleapis.com/auth/calendar",
                "https://www.googleapis.com/auth/calendar.events.readonly",
            ]);

        let authorize_url_with_offline = format!("{}&access_type=offline", authorize_url);
        println!("Opening: {}", authorize_url_with_offline.to_string());
        webbrowser::open(authorize_url_with_offline.as_str()).expect("failed to open web browser");

        let mut token = None;

        // A very naive implementation of the redirect server.
        let listener = TcpListener::bind("127.0.0.1:3003").unwrap();
        for stream in listener.incoming() {
            if let Ok(mut stream) = stream {
                let code;
                let state;
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
                    state = CsrfToken::new(value.into_owned());
                }

                let message = "<html><body>
                <script type=\"text/javascript\">
                  window.close() ;
                </script> 
                </body></html>";
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
