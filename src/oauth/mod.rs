pub mod google;
pub mod microsoft;

use oauth2::{basic::BasicClient, TokenResponse};
use oauth2::{
    AuthType, AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, PkceCodeChallenge,
    RedirectUrl, Scope, TokenUrl,
};
use std::io::{BufRead, BufReader, Write};

pub trait OauthClient {
    fn get_authorization_url(
        &self,
        scopes: Vec<&str>,
    ) -> (
        oauth2::url::Url,
        oauth2::CsrfToken,
        oauth2::PkceCodeVerifier,
    );
}

impl OauthClient for BasicClient {
    fn get_authorization_url(
        &self,
        scopes: Vec<&str>,
    ) -> (
        oauth2::url::Url,
        oauth2::CsrfToken,
        oauth2::PkceCodeVerifier,
    ) {
        // Proof Key for Code Exchange (PKCE - https://oauth.net/2/pkce/).
        // Create a PKCE code verifier and SHA-256 encode it as a code challenge.
        let (pkce_code_challenge, pkce_code_verifier) = PkceCodeChallenge::new_random_sha256();

        let s = scopes
            .iter()
            .map(|f| Scope::new(f.to_string()))
            .collect::<Vec<_>>();
        let auth_request = self
            .authorize_url(CsrfToken::new_random)
            .add_scopes(s.into_iter());

        // Generate the authorization URL to which we'll redirect the user.
        let (authorize_url, csrf_state) =
            auth_request.set_pkce_challenge(pkce_code_challenge).url();

        (authorize_url, csrf_state, pkce_code_verifier)
    }
}
