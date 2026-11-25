use std::{path::Path, sync::Arc};

use anyhow::{Context as _, Result};
use p5099_discord::{Backend, Config, Reply};
use p5099_server::{Caller, OperatorConsole};

pub fn configure(profile_root: &Path) -> Result<Option<Config>> {
    let config = Config::from_env(profile_root).context("Discord plugin settings")?;
    if config.is_none() {
        tracing::info!("Discord plugin is off; set P5099_DISCORD_TOKEN to enable it");
    }
    Ok(config)
}

pub fn start(config: Config, console: OperatorConsole) {
    tokio::spawn(async move {
        if let Err(error) = p5099_discord::run(config, Arc::new(InProcess(console))).await {
            tracing::error!(error = format!("{error:#}"), "Discord plugin stopped");
        }
    });
}

struct InProcess(OperatorConsole);

impl Backend for InProcess {
    fn execute(&self, caller: String, line: String) -> Reply {
        let console = self.0.clone();
        Box::pin(async move { Ok(console.execute(&Caller::Named(caller), &line).await) })
    }
}
