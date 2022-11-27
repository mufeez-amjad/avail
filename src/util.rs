use std::fs;

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