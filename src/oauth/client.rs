use oauth2::basic::BasicTokenType;
use oauth2::url::Url;
use oauth2::{basic::BasicClient, revocation::StandardRevocableToken, TokenResponse};
// Alternatively, this can be oauth2::curl::http_client or a custom.
use oauth2::reqwest::async_http_client;
use oauth2::{
    AuthType, AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, PkceCodeChallenge,
    RedirectUrl, RevocationUrl, Scope, TokenUrl, StandardTokenResponse, EmptyExtraTokenFields,
};
use std::env;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

use webbrowser;

pub trait OauthClient {
    fn get_authorization_url(&self, scopes: Vec<&str>) -> (oauth2::url::Url, oauth2::CsrfToken, oauth2::PkceCodeVerifier);
}

impl OauthClient for BasicClient {
    fn get_authorization_url(&self, scopes: Vec<&str>) -> (oauth2::url::Url, oauth2::CsrfToken, oauth2::PkceCodeVerifier) {
        // Proof Key for Code Exchange (PKCE - https://oauth.net/2/pkce/).
        // Create a PKCE code verifier and SHA-256 encode it as a code challenge.
        let (pkce_code_challenge, pkce_code_verifier) = PkceCodeChallenge::new_random_sha256();

        let s = scopes.iter().map(|f| Scope::new(f.to_string())).collect::<Vec<_>>();
        let auth_request = self.authorize_url(CsrfToken::new_random).add_scopes(s.into_iter());

        // Generate the authorization URL to which we'll redirect the user.
        let (authorize_url, csrf_state) =
            auth_request.set_pkce_challenge(pkce_code_challenge).url();

        (authorize_url, csrf_state, pkce_code_verifier)
    }
}

pub struct MicrosoftOauthClient {
    inner: BasicClient,
}

impl MicrosoftOauthClient {
    pub fn new(client_id: &str, client_secret: &str, _auth_url: &str, _token_url: &str) -> Self {
        let auth_url = AuthUrl::new("https://login.microsoftonline.com/common/oauth2/v2.0/authorize".to_string()).expect("Invalid authorization endpoint URL");
        let token_url = TokenUrl::new("https://login.microsoftonline.com/common/oauth2/v2.0/token".to_string()).expect("Invalid token endpoint URL");

        let client = BasicClient::new(
            ClientId::new(client_id.to_string()),
            Some(ClientSecret::new(client_secret.to_string())),
            auth_url,
            Some(token_url),
        )
        .set_auth_type(AuthType::RequestBody)
        .set_redirect_uri(
            RedirectUrl::new("http://localhost:3003/redirect".to_string()).expect("Invalid redirect URL"),
        );
        
        Self { inner: client }
    }

    pub async fn get_authorization_code(&self) -> oauth2::AccessToken {
        let (authorize_url, csrf_state, pkce_code_verifier) = self
            .inner
            .get_authorization_url(vec!["https://graph.microsoft.com/Calendars.Read", "https://graph.microsoft.com/User.Read"]); // "https://graph.microsoft.com/Calendars.Write", "https://graph.microsoft.com/User.Read"

        println!("{}", authorize_url);

        webbrowser::open(authorize_url.as_str()).expect("failed to open web browser");

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
                    println!("{}", url.to_string());

                    let code_pair = url
                        .query_pairs()
                        .find(|pair| {
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

                let message = "Go back to your terminal :)";
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n{}",
                    message.len(),
                    message
                );
                stream.write_all(response.as_bytes()).unwrap();

                println!("MS Graph returned the following code:\n{}\n", code.secret());
                println!(
                    "MS Graph returned the following state:\n{} (expected `{}`)\n",
                    state.secret(),
                    csrf_state.secret()
                );

                // Exchange the code with a token.
                let token_result = self.inner
                    .exchange_code(code)
                    // Send the PKCE code verifier in the token request
                    .set_pkce_verifier(pkce_code_verifier)
                    .request_async(async_http_client)
                    .await;

                token = Some(token_result.unwrap().access_token().clone());

                // The server will terminate itself after collecting the first code.
                break;
            }
        }

        return token.unwrap();
    }
}


