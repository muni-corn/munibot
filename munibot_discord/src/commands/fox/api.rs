use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct RandomFoxResponse {
    pub image: String,

    #[allow(dead_code)]
    pub link: String,
}

const RANDOMFOX_API_URL: &str = "https://randomfox.ca/floof";

pub(super) async fn get_fox() -> Result<RandomFoxResponse, reqwest::Error> {
    reqwest::get(RANDOMFOX_API_URL).await?.json().await
}
