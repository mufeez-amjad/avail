use super::OauthClient;

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://www.googleapis.com/oauth2/v3/token";
const REDIRECT_URL: &str = "http://localhost:3003/redirect";

pub fn new_client(client_id: &str, client_secret: &str) -> OauthClient {
    OauthClient::new(
        client_id,
        client_secret,
        vec!["https://www.googleapis.com/auth/calendar"],
        AUTH_URL,
        TOKEN_URL,
        REDIRECT_URL,
    )
}
