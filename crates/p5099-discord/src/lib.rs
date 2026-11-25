mod actions;
mod bot;
mod commands;
mod components;
mod config;
mod console;
mod format;
mod monitor;
mod views;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use anyhow::{Context as _, Result};
use serenity::{
    Client,
    model::{gateway::GatewayIntents, user::OnlineStatus},
    secrets::Token,
};
use tracing::info;

use crate::{bot::Bot, console::Console, monitor::Monitor};
pub use crate::{
    config::Config,
    console::{Backend, ConsoleError, ConsoleResult, Reply},
};

pub async fn run(config: Config, backend: Arc<dyn Backend>) -> Result<()> {
    let token: Token = config
        .discord_token
        .parse()
        .context("P5099_DISCORD_TOKEN is not a bot token")?;
    info!(guild_id = %config.guild_id, "starting the Discord plugin");
    let monitor = Arc::new(Monitor::new(Console::new(backend), config.state_file).await);
    let bot = Arc::new(Bot::new(monitor, config.guild_id));

    let mut client = Client::builder(token, GatewayIntents::GUILDS)
        .event_handler(Arc::new(bot::Handler { bot }))
        .status(OnlineStatus::Online)
        .await
        .context("failed to create the Discord client")?;
    client.start().await.context("Discord client stopped")
}
