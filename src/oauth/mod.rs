pub mod google;
pub mod microsoft;

use oauth2::{
    basic::BasicClient, reqwest::async_http_client, AuthType, AuthUrl, AuthorizationCode, ClientId,
    ClientSecret, CsrfToken, PkceCodeChallenge, RedirectUrl, Scope, TokenResponse, TokenUrl,
};
use reqwest::Url;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

pub struct OauthClient {
    pub(crate) inner: BasicClient,
    pub client_id: String,
    pub client_secret: String,
    pub scopes: Vec<String>,
}

impl OauthClient {
    pub(crate) fn new(
        client_id: &str,
        client_secret: &str,
        scopes: Vec<&str>,
        auth_url: &str,
        token_url: &str,
        redirect_url: &str,
    ) -> Self {
        let auth_url =
            AuthUrl::new(auth_url.to_string()).expect("Invalid authorization endpoint URL");
        let token_url = TokenUrl::new(token_url.to_string()).expect("Invalid token endpoint URL");

        let client = BasicClient::new(
            ClientId::new(client_id.to_string()),
            Some(ClientSecret::new(client_secret.to_string())),
            auth_url,
            Some(token_url),
        )
        .set_auth_type(AuthType::RequestBody)
        .set_redirect_uri(
            RedirectUrl::new(redirect_url.to_string()).expect("Invalid redirect URL"),
        );

        Self {
            inner: client,
            client_id: client_id.to_string(),
            client_secret: client_secret.to_string(),
            scopes: scopes.iter().map(|f| f.to_string()).collect(),
        }
    }

    fn get_authorization_url(
        &self,
    ) -> (
        oauth2::url::Url,
        oauth2::CsrfToken,
        oauth2::PkceCodeVerifier,
    ) {
        // Proof Key for Code Exchange (PKCE - https://oauth.net/2/pkce/).
        // Create a PKCE code verifier and SHA-256 encode it as a code challenge.
        let (pkce_code_challenge, pkce_code_verifier) = PkceCodeChallenge::new_random_sha256();

        let s = self.scopes.iter().map(|f| Scope::new(f.to_string()));

        let auth_request = self
            .inner
            .authorize_url(CsrfToken::new_random)
            .add_scopes(s);

        // Generate the authorization URL to which we'll redirect the user.
        let (authorize_url, csrf_state) =
            auth_request.set_pkce_challenge(pkce_code_challenge).url();

        (authorize_url, csrf_state, pkce_code_verifier)
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

    pub async fn get_authorization_code(
        &self,
        shutdown_receiver: tokio::sync::oneshot::Receiver<()>,
    ) -> (String, String) {
        let (authorize_url, _csrf_state, pkce_code_verifier) = self.get_authorization_url();

        let authorize_url_with_offline = format!("{}&access_type=offline", authorize_url);
        println!("Opening browser to {}", authorize_url_with_offline);

        webbrowser::open(authorize_url_with_offline.as_str()).expect("failed to open web browser");

        // A very naive implementation of the redirect server.
        let listener = TcpListener::bind("127.0.0.1:3003").await.unwrap();

        let handle: JoinHandle<Result<Option<AuthorizationCode>, _>> = tokio::spawn(async move {
            loop {
                tokio::select! {
                    conn = listener.accept() => {
                        // Process the connection
                        match conn {
                            Ok((stream, _addr)) => {
                                return process_stream(stream).await;
                            }
                            Err(e) => {
                                // An error occurred, so log it and continue
                                println!("Error accepting connection: {:?}", e);
                                return Err(anyhow::anyhow!("Error accepting connection {:?}", e));
                            }
                        }
                    }
                    _ = shutdown_receiver => {
                        // The shutdown signal has been received, so break out of the loop
                        // and shutdown the TcpListener
                        return Ok(None)
                    }
                };
            }
        });

        let code = match handle.await.unwrap() {
            Ok(c) => c.expect("failed to retrieve authorization code"),
            Err(e) => {
                panic!("{:?}", e);
            }
        };

        // Exchange the code with a token.
        let token_result = self
            .inner
            .exchange_code(code)
            // Send the PKCE code verifier in the token request
            .set_pkce_verifier(pkce_code_verifier)
            .request_async(async_http_client)
            .await;

        let inner = token_result.unwrap();
        let access_token = inner.access_token().secret().to_owned();
        let refresh_token = inner.refresh_token().unwrap().secret().to_owned();
        (access_token, refresh_token)
    }
}

trait OauthTokenRetriever {
    fn get_authorization_code(&self) -> (String, String);
    fn refresh_access_token(&self, refresh_token: String) -> String;
}

async fn process_stream(mut stream: TcpStream) -> anyhow::Result<Option<AuthorizationCode>> {
    let code;
    let _state;
    let code = {
        let mut request_line = String::new();
        let _ = stream.readable().await;
        stream.read_to_string(&mut request_line).await?;

        let redirect_url = request_line.split_whitespace().nth(1).unwrap();
        let url = Url::parse(&("http://localhost".to_string() + redirect_url)).unwrap();

        let code_pair = url.query_pairs().find(|pair| {
            let &(ref key, _) = pair;
            key == "code"
        });

        if code_pair.is_none() {
            return Err(anyhow::anyhow!("Code pair was not received"));
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

        code
    };

    let message = "Go back to your terminal :)";
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n{}",
        message.len(),
        message
    );
    stream.write_all_buf(&mut response.as_bytes()).await?;

    // The server will terminate itself after collecting the first code.
    Ok(Some(code))
}
