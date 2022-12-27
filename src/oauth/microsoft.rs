use super::OauthClient;

const AUTH_URL: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/authorize";
const TOKEN_URL: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/token";
const REDIRECT_URL: &str = "http://localhost:3003/redirect";

pub fn new_client(client_id: &str, _client_secret: &str) -> OauthClient {
    OauthClient::new(
        client_id,
        // "AADSTS90023: Public clients can't send a client secret.
        "",
        vec![
            "https://graph.microsoft.com/Calendars.ReadWrite",
            "https://graph.microsoft.com/User.Read",
            "offline_access",
        ],
        AUTH_URL,
        TOKEN_URL,
        REDIRECT_URL,
    )
}
