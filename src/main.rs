mod oauth;

use oauth::client::MicrosoftOauthClient;
use dialoguer::{Input, MultiSelect};

#[derive(serde::Deserialize, Clone)]
struct Calendar {
    id: String,
    name: String,

    #[serde(default)]
    selected: bool,
}

#[derive(serde::Deserialize)]
struct GraphResponse<T> {
    value: Vec<T>
}

async fn get_calendars(token: String) -> Result<Vec<Calendar>, Box<dyn std::error::Error>> {
    let resp: GraphResponse<Calendar> = reqwest::Client::new()
        .get("https://graph.microsoft.com/v1.0/me/calendars")
        .bearer_auth(token)
        .header("Content-Type", "application/json")
        .send()
        .await
        .unwrap()
        .json()
        .await?;

    let calendars = resp.value;

    let calendar_names: Vec<String> = calendars.iter().map(|cal| cal.name.to_owned()).collect(); 
    
    let selected_calendars_idx : Vec<usize> = MultiSelect::new()
    .items(&calendar_names)
    .with_prompt("Select the calendars you want to use:")
    .interact()?;

    Ok(selected_calendars_idx.iter().map(|idx| calendars[*idx].clone()).collect())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = MicrosoftOauthClient::new("345ac594-c15f-4904-b9c5-49a29016a8d2", "", "", "");
    let token = client.get_authorization_code().await.secret().to_owned();
    println!("token: {}", token);

    let calendars = get_calendars(token).await?;
    println!("{}", calendars.len());

    let input : String = Input::new()
        .with_prompt("Tea or coffee?")
        .with_initial_text("Yes")
        .default("No".into())
        .interact_text()?;
    
    Ok(())
}

