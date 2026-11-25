use std::{
    collections::HashSet,
    sync::{
        Arc, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
};

use serenity::{
    async_trait,
    builder::{
        CreateAllowedMentions, CreateInteractionResponse, CreateInteractionResponseMessage,
        EditInteractionResponse,
    },
    http::Http,
    model::{
        application::Interaction, channel::MessageFlags, event::FullEvent, guild::Member,
        id::GuildId, permissions::Permissions, user::User,
    },
    prelude::{Context, EventHandler},
};
use tracing::{error, info, warn};

use crate::{
    actions::Action,
    commands, components,
    console::Console,
    monitor::{Monitor, post},
    views::{self, Components, Notice, Tone},
};

pub struct Bot {
    pub monitor: Arc<Monitor>,
    guild: GuildId,
    started: AtomicBool,
    registered: std::sync::Mutex<HashSet<GuildId>>,
}

impl Bot {
    pub fn new(monitor: Arc<Monitor>, guild: GuildId) -> Self {
        Self {
            monitor,
            guild,
            started: AtomicBool::new(false),
            registered: std::sync::Mutex::new(HashSet::new()),
        }
    }

    pub fn console(&self) -> &Console {
        self.monitor.console()
    }

    pub fn console_for(&self, user: &User) -> Console {
        self.monitor.console().acting_as(user)
    }

    pub fn refresh_soon(&self, http: &Arc<Http>) {
        let monitor = Arc::clone(&self.monitor);
        let http = Arc::clone(http);
        tokio::spawn(async move { monitor.poll(&http).await });
    }

    pub async fn record(&self, http: &Arc<Http>, user: &User, action: Action) -> Notice {
        let Action { label, outcome } = action;
        let notice = match &outcome {
            Ok(summary) => {
                info!(user = %user.name, user_id = %user.id, action = label, summary, "action");
                Notice::new(Tone::Success, summary.clone())
            }
            Err(error) => {
                warn!(user = %user.name, user_id = %user.id, action = label, error, "action failed");
                Notice::new(Tone::Error, error.clone())
            }
        };
        if let Some(channel) = self.monitor.settings().audit_channel {
            let card = views::message(
                notice.tone,
                format!("**{label}**\n{}\n-# <@{}>", notice.text, user.id),
            );
            post(http, channel, card).await;
        }
        if outcome.is_ok() {
            self.refresh_soon(http);
        }
        notice
    }
}

pub fn new_message(components: Components, ephemeral: bool) -> CreateInteractionResponse<'static> {
    let mut flags = MessageFlags::IS_COMPONENTS_V2;
    if ephemeral {
        flags |= MessageFlags::EPHEMERAL;
    }
    CreateInteractionResponse::Message(
        CreateInteractionResponseMessage::new()
            .components(components)
            .flags(flags)
            .allowed_mentions(CreateAllowedMentions::new()),
    )
}

pub fn update_message(components: Components) -> CreateInteractionResponse<'static> {
    CreateInteractionResponse::UpdateMessage(
        CreateInteractionResponseMessage::new()
            .components(components)
            .flags(MessageFlags::IS_COMPONENTS_V2)
            .allowed_mentions(CreateAllowedMentions::new()),
    )
}

pub fn edit(components: Components) -> EditInteractionResponse<'static> {
    EditInteractionResponse::new()
        .components(components)
        .flags(MessageFlags::IS_COMPONENTS_V2)
        .allowed_mentions(CreateAllowedMentions::new())
}

pub const OPERATOR_PERMISSIONS: Permissions = Permissions::MANAGE_GUILD;

pub const PUBLIC_COMMANDS: &[&str] = &["status", "online", "rooms", "ping", "help"];
const PUBLIC_COMPONENTS: &[&str] = &["dash:refresh", "dash:players", "dash:rooms"];

fn holds(member: Option<&Member>, required: Permissions) -> bool {
    member
        .and_then(|member| member.permissions)
        .is_some_and(|held| held.contains(Permissions::ADMINISTRATOR) || held.contains(required))
}

pub fn authorized(guild: GuildId, interaction: &Interaction) -> bool {
    if interaction.guild_id() != Some(guild) {
        return false;
    }
    let (member, public, required) = match interaction {
        Interaction::Command(command) => {
            let name = command.data.name.as_str();
            let required = if name == "purge" {
                Permissions::MANAGE_MESSAGES
            } else {
                OPERATOR_PERMISSIONS
            };
            (
                command.member.as_deref(),
                PUBLIC_COMMANDS.contains(&name),
                required,
            )
        }
        Interaction::Autocomplete(command) => {
            (command.member.as_deref(), false, OPERATOR_PERMISSIONS)
        }
        Interaction::Component(component) => (
            component.member.as_deref(),
            PUBLIC_COMPONENTS.contains(&component.data.custom_id.as_str()),
            OPERATOR_PERMISSIONS,
        ),
        Interaction::Modal(modal) => (modal.member.as_deref(), false, OPERATOR_PERMISSIONS),
        _ => return false,
    };
    public || holds(member, required)
}

async fn refuse(ctx: &Context, interaction: &Interaction) {
    let response = new_message(
        views::message(
            Tone::Error,
            "이 작업은 서버 관리 권한이 있는 운영자만 할 수 있습니다",
        ),
        true,
    );
    let result = match interaction {
        Interaction::Command(command) => command.create_response(&ctx.http, response).await,
        Interaction::Component(component) => component.create_response(&ctx.http, response).await,
        Interaction::Modal(modal) => modal.create_response(&ctx.http, response).await,
        _ => return,
    };
    log_failure(result, "refusal");
}

pub fn log_failure(result: serenity::Result<impl Sized>, what: &str) {
    if let Err(error) = result {
        warn!(%error, what, "failed to answer an interaction");
    }
}

pub struct Handler {
    pub bot: Arc<Bot>,
}

#[async_trait]
impl EventHandler for Handler {
    async fn dispatch(&self, ctx: &Context, event: &FullEvent) {
        match event {
            FullEvent::Ready { data_about_bot, .. } => {
                info!(user = %data_about_bot.user.name, "connected to Discord");
                self.bot.monitor.set_context(ctx.clone());
                if self.bot.started.swap(true, Ordering::SeqCst) {
                    return;
                }
                let monitor = Arc::clone(&self.bot.monitor);
                tokio::spawn(monitor.run(Arc::clone(&ctx.http)));
            }
            FullEvent::GuildCreate { guild, .. } => {
                if guild.id != self.bot.guild {
                    warn!(
                        guild = %guild.name,
                        guild_id = %guild.id,
                        "leaving a Discord server other than P5099_DISCORD_GUILD_ID"
                    );
                    if let Err(error) = guild.id.leave(&ctx.http).await {
                        error!(%error, guild_id = %guild.id, "failed to leave the server");
                    }
                    return;
                }
                let new = self
                    .bot
                    .registered
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(guild.id);
                if !new {
                    return;
                }
                match guild
                    .id
                    .set_commands(&ctx.http, &commands::definitions())
                    .await
                {
                    Ok(commands) => {
                        info!(guild = %guild.name, count = commands.len(), "registered commands");
                    }
                    Err(error) => {
                        error!(%error, guild = %guild.name, "failed to register commands");
                        self.bot
                            .registered
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .remove(&guild.id);
                    }
                }
            }
            FullEvent::InteractionCreate { interaction, .. }
                if !authorized(self.bot.guild, interaction) =>
            {
                let user = match interaction {
                    Interaction::Command(i) | Interaction::Autocomplete(i) => Some(&i.user),
                    Interaction::Component(i) => Some(&i.user),
                    Interaction::Modal(i) => Some(&i.user),
                    _ => None,
                };
                warn!(
                    user = ?user.map(|user| &user.name),
                    user_id = ?user.map(|user| user.id),
                    guild_id = ?interaction.guild_id(),
                    "refused an interaction"
                );
                if interaction.guild_id() == Some(self.bot.guild) {
                    refuse(ctx, interaction).await;
                }
            }
            FullEvent::InteractionCreate { interaction, .. } => match interaction {
                Interaction::Command(command) => commands::run(&self.bot, ctx, command).await,
                Interaction::Autocomplete(command) => {
                    commands::autocomplete(&self.bot, ctx, command).await;
                }
                Interaction::Component(component) => {
                    components::component(&self.bot, ctx, component).await;
                }
                Interaction::Modal(modal) => components::modal(&self.bot, ctx, modal).await,
                _ => {}
            },
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    const GUILD: u64 = 1_000_000_000_000_001;
    const OTHER_GUILD: u64 = 1_000_000_000_000_002;
    const MANAGE_GUILD: &str = "32";
    const ADMINISTRATOR: &str = "8";
    const MANAGE_MESSAGES: &str = "8192";
    const MEMBER: &str = "3072";

    fn interaction(kind: u8, data: &Value, guild: u64, permissions: &str) -> Interaction {
        let user = json!({"id": "42", "username": "rider", "discriminator": "0", "global_name": null, "avatar": null});
        let value = json!({
            "id": "10",
            "application_id": "11",
            "type": kind,
            "data": data,
            "guild_id": guild.to_string(),
            "channel_id": "12",
            "channel": {"id": "12", "type": 0, "guild_id": guild.to_string()},
            "member": {
                "user": user,
                "roles": [],
                "joined_at": "2026-01-01T00:00:00.000000+00:00",
                "deaf": false,
                "mute": false,
                "flags": 0,
                "permissions": permissions,
            },
            "token": "token",
            "version": 1,
            "locale": "ko",
            "guild_locale": "ko",
            "app_permissions": "0",
            "entitlements": [],
            "authorizing_integration_owners": {},
            "attachment_size_limit": 0,
            "message": {
                "id": "13", "channel_id": "12", "author": user, "content": "",
                "timestamp": "2026-01-01T00:00:00.000000+00:00", "edited_timestamp": null,
                "tts": false, "mention_everyone": false, "mentions": [], "mention_roles": [],
                "attachments": [], "embeds": [], "pinned": false, "type": 0,
            },
        });
        serde_json::from_str(&value.to_string()).expect("interaction JSON")
    }

    fn command(name: &str, guild: u64, permissions: &str) -> Interaction {
        interaction(
            2,
            &json!({"id": "20", "name": name, "type": 1}),
            guild,
            permissions,
        )
    }

    fn button(custom_id: &str, permissions: &str) -> Interaction {
        interaction(
            3,
            &json!({"custom_id": custom_id, "component_type": 2}),
            GUILD,
            permissions,
        )
    }

    fn allowed(interaction: &Interaction) -> bool {
        authorized(GuildId::new(GUILD), interaction)
    }

    #[test]
    fn members_get_only_public_views() {
        for name in PUBLIC_COMMANDS {
            assert!(allowed(&command(name, GUILD, MEMBER)), "{name}");
        }
        for name in [
            "shutdown", "account", "ban", "players", "settings", "panel", "purge",
        ] {
            assert!(!allowed(&command(name, GUILD, MEMBER)), "{name}");
        }
        for id in PUBLIC_COMPONENTS {
            assert!(allowed(&button(id, MEMBER)), "{id}");
        }
        for id in [
            "dash:panel",
            "cfm:shutdown",
            "panel:maint:on",
            "acct:pw:Rider",
        ] {
            assert!(!allowed(&button(id, MEMBER)), "{id}");
        }
    }

    #[test]
    fn operators_need_manage_server() {
        for permissions in [MANAGE_GUILD, ADMINISTRATOR] {
            assert!(allowed(&command("shutdown", GUILD, permissions)));
            assert!(allowed(&button("cfm:shutdown", permissions)));
        }
        assert!(!allowed(&command("shutdown", GUILD, MANAGE_MESSAGES)));
        assert!(allowed(&command("purge", GUILD, MANAGE_MESSAGES)));
    }

    #[test]
    fn other_servers_get_nothing() {
        assert!(!allowed(&command("status", OTHER_GUILD, ADMINISTRATOR)));
        assert!(!allowed(&command("shutdown", OTHER_GUILD, ADMINISTRATOR)));
    }
}
