use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

pub fn get_avail_directory() -> anyhow::Result<String> {
    let home_dir = dirs::home_dir();
    if home_dir.is_none() {
        return Err(anyhow::anyhow!("unable to get home directory"));
    }
    let home_dir_str = home_dir.unwrap().to_str().unwrap().to_string();

    let avail_dir = format!("{}/.avail", home_dir_str);

    // Create if doesn't exist
    fs::create_dir_all(&avail_dir)?;

    Ok(avail_dir)
}

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct OAuthConfig {
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct AvailConfig {
    pub google: OAuthConfig,
    pub microsoft: OAuthConfig,
}

pub fn load_config() -> anyhow::Result<AvailConfig> {
    let str_path = format!("{}/conf.toml", get_avail_directory()?);
    let config_path = Path::new(&str_path);
    if config_path.exists() {
        let cfg: AvailConfig = toml::from_str(&fs::read_to_string(config_path)?)?;
        if (cfg.google.client_id.is_empty() || cfg.google.client_secret.is_empty())
            && (cfg.microsoft.client_id.is_empty() || cfg.microsoft.client_secret.is_empty())
        {
            return Err(anyhow::anyhow!(format!(
                "Please ensure {} is configured correctly",
                str_path
            )));
        }
        Ok(cfg)
    } else {
        fs::create_dir_all(config_path.parent().unwrap_or_else(|| Path::new("")))?;

        let mut buffer = File::create(config_path)?;
        buffer.write_all(toml::to_string_pretty(&AvailConfig::default())?.as_bytes())?;
        Err(anyhow::anyhow!(format!(
            "Please add your OAuth keys to {}",
            str_path
        )))
    }
}
