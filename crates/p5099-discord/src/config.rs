use std::{env, path::PathBuf};

use anyhow::{Context, Result};
use serenity::model::id::GuildId;

pub const STATE_FILE_NAME: &str = "Discord.json";

pub struct Config {
    pub discord_token: String,
    pub guild_id: GuildId,
    pub state_file: PathBuf,
}

impl Config {
    pub fn from_env(profile_root: &std::path::Path) -> Result<Option<Self>> {
        let Some(discord_token) = optional("P5099_DISCORD_TOKEN") else {
            return Ok(None);
        };
        let guild_id = parse_guild_id(&required("P5099_DISCORD_GUILD_ID")?)?;
        Ok(Some(Self {
            discord_token,
            guild_id,
            state_file: optional("P5099_DISCORD_STATE_FILE")
                .map_or_else(|| profile_root.join(STATE_FILE_NAME), PathBuf::from),
        }))
    }
}

fn parse_guild_id(text: &str) -> Result<GuildId> {
    text.parse::<u64>()
        .ok()
        .filter(|id| *id != 0)
        .map(GuildId::new)
        .with_context(|| format!("P5099_DISCORD_GUILD_ID `{text}` is not a Discord server ID"))
}

fn optional(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn required(name: &str) -> Result<String> {
    optional(name).with_context(|| format!("set {name}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_guild_ids() {
        assert_eq!(
            parse_guild_id("123456789012345678").unwrap(),
            GuildId::new(123_456_789_012_345_678)
        );
        for bad in ["0", "", "abc", "-1", "12 34"] {
            assert!(parse_guild_id(bad).is_err(), "{bad}");
        }
    }
}
