use std::collections::HashMap;

use p5099_core::account_protocol::{MAX_PASSWORD_CHARS, MIN_PASSWORD_CHARS};
use serenity::{
    builder::{
        CreateInputText, CreateInteractionResponse, CreateLabel, CreateModal, CreateModalComponent,
    },
    http::Http,
    model::application::{
        ComponentInteraction, ComponentInteractionDataKind, InputTextStyle, LabelComponent,
        ModalComponent, ModalInteraction, ModalInteractionData,
    },
    model::id::GenericChannelId,
    prelude::Context,
};

use crate::{
    actions::{self, Action},
    bot::{Bot, edit, log_failure, new_message, update_message},
    console::Console,
    format::{escape, on_off},
    monitor::{Health, POLL_CHOICES, try_post},
    views::{self, AccountFilter, Components, Notice, Tone},
};

pub async fn panel(console: &Console, notice: Option<&Notice>) -> Components {
    match tokio::try_join!(
        console.status(),
        console.players(),
        console.rewards(),
        console.throttles()
    ) {
        Ok((status, players, rewards, throttles)) => {
            views::panel(&status, &players, &rewards, &throttles, notice)
        }
        Err(error) => {
            let mut text = error.to_string();
            if let Some(notice) = notice {
                text = format!("{}\n\n{text}", notice.text);
            }
            views::error(text)
        }
    }
}

pub async fn account_card(
    console: &Console,
    nickname: &str,
    notice: Option<&Notice>,
) -> Components {
    match console.account_info(nickname).await {
        Ok(info) => views::account(&info, notice),
        Err(error) => match notice {
            Some(notice) => views::notice(notice),
            None => views::error(error.to_string()),
        },
    }
}

fn modal_title(action: &str, nickname: &str) -> String {
    format!("{action} · {nickname}").chars().take(45).collect()
}

pub fn password_modal(custom_id: String, action: &str, nickname: &str) -> CreateModal<'static> {
    let input = |id: &'static str| {
        CreateInputText::new(InputTextStyle::Short, id)
            .min_length(u16::try_from(MIN_PASSWORD_CHARS).unwrap_or(u16::MAX))
            .max_length(u16::try_from(MAX_PASSWORD_CHARS).unwrap_or(u16::MAX))
            .required(true)
    };
    CreateModal::new(custom_id, modal_title(action, nickname)).components(vec![
        CreateModalComponent::Label(CreateLabel::input_text("비밀번호", input("password"))),
        CreateModalComponent::Label(CreateLabel::input_text("비밀번호 확인", input("confirm"))),
    ])
}

fn ban_modal(nickname: &str) -> CreateModal<'static> {
    CreateModal::new(
        format!("modal:ban:{nickname}"),
        modal_title("정지", nickname),
    )
    .components(vec![
        CreateModalComponent::Label(CreateLabel::input_text(
            "사유",
            CreateInputText::new(InputTextStyle::Short, "reason")
                .max_length(200)
                .required(false),
        )),
        CreateModalComponent::Label(CreateLabel::input_text(
            "기간",
            CreateInputText::new(InputTextStyle::Short, "duration")
                .placeholder("30m, 12h, 7d (비우면 무기한)")
                .max_length(16)
                .required(false),
        )),
    ])
}

pub fn kick_all_confirm(keep_admins: bool) -> Components {
    if keep_admins {
        views::confirm("관리자 외 전원 퇴장할까요?", "kickall:1")
    } else {
        views::confirm("전원 퇴장할까요?", "kickall:0")
    }
}

pub fn shutdown_confirm() -> Components {
    views::confirm("서버를 종료할까요?", "shutdown")
}

pub fn delete_confirm(nickname: &str) -> Components {
    views::confirm(
        &format!("{} 계정을 삭제할까요?", escape(nickname)),
        &format!("delete:{nickname}"),
    )
}

pub async fn component(bot: &Bot, ctx: &Context, interaction: &ComponentInteraction) {
    let http = &ctx.http;
    let id = interaction.data.custom_id.as_str();
    let (scope, rest) = id.split_once(':').unwrap_or((id, ""));
    match scope {
        "dash" => dashboard_button(bot, ctx, interaction, rest).await,
        "panel" => panel_button(bot, ctx, interaction, rest).await,
        "acct" => account_button(bot, ctx, interaction, rest).await,
        "cfm" => confirmed(bot, ctx, interaction, rest).await,
        "set" => settings_button(bot, ctx, interaction, rest).await,
        "rewards" | "throttles" => {
            if interaction.defer(http).await.is_err() {
                return;
            }
            let action = if scope == "rewards" {
                actions::retry_rewards(&bot.console_for(&interaction.user), None).await
            } else {
                actions::clear_throttles(&bot.console_for(&interaction.user), None).await
            };
            let notice = bot.record(http, &interaction.user, action).await;
            log_failure(
                interaction
                    .edit_response(http, edit(views::notice(&notice)))
                    .await,
                id,
            );
        }
        _ => {}
    }
}

async fn settings_button(bot: &Bot, ctx: &Context, interaction: &ComponentInteraction, rest: &str) {
    let http = &ctx.http;
    if interaction.defer(http).await.is_err() {
        return;
    }
    let action = match (rest, &interaction.data.kind) {
        ("alert" | "audit", ComponentInteractionDataKind::ChannelSelect { values }) => Some(
            set_channel(
                bot,
                http,
                rest,
                values.first().map(|channel| channel.widen()),
            )
            .await,
        ),
        ("alert:off", _) => Some(set_channel(bot, http, "alert", None).await),
        ("audit:off", _) => Some(set_channel(bot, http, "audit", None).await),
        ("join:1" | "join:0", _) => {
            let enabled = rest == "join:1";
            bot.monitor
                .update_settings(|settings| settings.join_alerts = enabled)
                .await;
            Some(Action {
                label: "접속·퇴장 알림".to_owned(),
                outcome: Ok(format!("접속·퇴장 알림 {}", on_off(enabled))),
            })
        }
        ("poll", ComponentInteractionDataKind::StringSelect { values }) => {
            match values.first().and_then(|value| value.parse::<u64>().ok()) {
                Some(seconds) if POLL_CHOICES.contains(&seconds) => {
                    bot.monitor
                        .update_settings(|settings| settings.poll_seconds = seconds)
                        .await;
                    Some(Action {
                        label: "갱신 간격".to_owned(),
                        outcome: Ok(format!("갱신 간격 {seconds}초")),
                    })
                }
                _ => None,
            }
        }
        _ => None,
    };
    let notice = match action {
        Some(action) => Some(bot.record(http, &interaction.user, action).await),
        None => None,
    };
    let components = views::settings(&bot.monitor.settings(), notice.as_ref());
    log_failure(
        interaction.edit_response(http, edit(components)).await,
        rest,
    );
}

async fn set_channel(
    bot: &Bot,
    http: &Http,
    which: &str,
    channel: Option<GenericChannelId>,
) -> Action {
    let name = if which == "alert" {
        "알림 채널"
    } else {
        "운영 기록 채널"
    };
    let label = channel.map_or_else(
        || format!("{name} 끄기"),
        |channel| format!("{name} <#{channel}>"),
    );
    if let Some(channel) = channel {
        let greeting = views::message(Tone::Info, format!("{name}로 설정됨"));
        if let Err(error) = try_post(http, channel, greeting).await {
            return Action::refused(
                label,
                format!("<#{channel}>에 메시지를 보낼 수 없습니다 ({error})"),
            );
        }
    }
    bot.monitor
        .update_settings(|settings| {
            if which == "alert" {
                settings.alert_channel = channel;
            } else {
                settings.audit_channel = channel;
            }
        })
        .await;
    Action {
        label,
        outcome: Ok(channel.map_or_else(
            || format!("{name} 꺼짐"),
            |channel| format!("{name} <#{channel}>"),
        )),
    }
}

async fn dashboard_button(
    bot: &Bot,
    ctx: &Context,
    interaction: &ComponentInteraction,
    rest: &str,
) {
    let http = &ctx.http;
    match rest {
        "panel" => {
            if interaction.defer_ephemeral(http).await.is_err() {
                return;
            }
            let components = panel(&bot.console_for(&interaction.user), None).await;
            log_failure(
                interaction.edit_response(http, edit(components)).await,
                rest,
            );
        }
        "refresh" => {
            if interaction.defer(http).await.is_err() {
                return;
            }
            bot.monitor.refresh(http).await;
            if !bot
                .monitor
                .dashboard_messages()
                .await
                .contains(&interaction.message.id)
            {
                let board = bot.monitor.board();
                let components = views::dashboard(&board.health, board.changed);
                log_failure(
                    interaction.edit_response(http, edit(components)).await,
                    rest,
                );
            }
        }
        "players" | "rooms" => {
            let components = match bot.monitor.board().health {
                Health::Online(snapshot) if rest == "players" => {
                    views::online(&snapshot.players, &snapshot.rooms)
                }
                Health::Online(snapshot) => views::rooms(&snapshot.rooms),
                Health::Offline { error, .. } => views::offline(&error),
                Health::Starting => views::message(Tone::Info, "확인 중"),
            };
            log_failure(
                interaction
                    .create_response(http, new_message(components, true))
                    .await,
                rest,
            );
        }
        _ => {}
    }
}

async fn panel_button(bot: &Bot, ctx: &Context, interaction: &ComponentInteraction, rest: &str) {
    let http = &ctx.http;
    let console = &bot.console_for(&interaction.user);
    let confirmation = match rest {
        "kickall:keep" => Some(kick_all_confirm(true)),
        "kickall:all" => Some(kick_all_confirm(false)),
        "shutdown" => Some(shutdown_confirm()),
        _ => None,
    };
    if let Some(components) = confirmation {
        log_failure(
            interaction
                .create_response(http, new_message(components, true))
                .await,
            rest,
        );
        return;
    }
    if interaction.defer(http).await.is_err() {
        return;
    }
    let action = match rest {
        "maint:on" => Some(actions::set_maintenance(console, true, false).await),
        "maint:off" => Some(actions::set_maintenance(console, false, false).await),
        "reg:open" => Some(actions::set_registration(console, true).await),
        "reg:close" => Some(actions::set_registration(console, false).await),
        "retry" => Some(actions::retry_rewards(console, None).await),
        "clearthr" => Some(actions::clear_throttles(console, None).await),
        "kick" => {
            if let ComponentInteractionDataKind::StringSelect { values } = &interaction.data.kind
                && let Some(nickname) = values.first()
            {
                Some(actions::kick(console, nickname).await)
            } else {
                None
            }
        }
        _ => None,
    };
    let notice = match action {
        Some(action) => Some(bot.record(http, &interaction.user, action).await),
        None => None,
    };
    let components = panel(console, notice.as_ref()).await;
    log_failure(
        interaction.edit_response(http, edit(components)).await,
        rest,
    );
}

async fn account_button(bot: &Bot, ctx: &Context, interaction: &ComponentInteraction, rest: &str) {
    let http = &ctx.http;
    let console = &bot.console_for(&interaction.user);
    let (action, argument) = rest.split_once(':').unwrap_or((rest, ""));
    let early = match action {
        "ban" => Some(CreateInteractionResponse::Modal(ban_modal(argument))),
        "pw" => Some(CreateInteractionResponse::Modal(password_modal(
            format!("modal:pw:{argument}"),
            "비밀번호 변경",
            argument,
        ))),
        "delete" => Some(new_message(delete_confirm(argument), true)),
        _ => None,
    };
    if let Some(response) = early {
        log_failure(interaction.create_response(http, response).await, rest);
        return;
    }
    if interaction.defer(http).await.is_err() {
        return;
    }
    let components = match action {
        "page" => {
            let (filter, page) = argument.split_once(':').unwrap_or(("all", "0"));
            match console.accounts().await {
                Ok(accounts) => views::accounts(
                    &accounts,
                    AccountFilter::parse(filter),
                    page.parse().unwrap_or(0),
                ),
                Err(error) => views::error(error.to_string()),
            }
        }
        "refresh" => account_card(console, argument, None).await,
        "kick" | "unban" | "admin" => {
            let (done, nickname) = match action {
                "kick" => (actions::kick(console, argument).await, argument),
                "unban" => (actions::unban(console, argument).await, argument),
                _ => {
                    let (admin, nickname) = argument.split_once(':').unwrap_or(("0", argument));
                    (
                        actions::set_admin(console, nickname, admin == "1").await,
                        nickname,
                    )
                }
            };
            let notice = bot.record(http, &interaction.user, done).await;
            account_card(console, nickname, Some(&notice)).await
        }
        _ => return,
    };
    log_failure(
        interaction.edit_response(http, edit(components)).await,
        rest,
    );
}

async fn confirmed(bot: &Bot, ctx: &Context, interaction: &ComponentInteraction, rest: &str) {
    let http = &ctx.http;
    let console = &bot.console_for(&interaction.user);
    if rest == "cancel" {
        log_failure(
            interaction
                .create_response(http, update_message(views::message(Tone::Info, "취소")))
                .await,
            rest,
        );
        return;
    }
    if interaction.defer(http).await.is_err() {
        return;
    }
    let (action, argument) = rest.split_once(':').unwrap_or((rest, ""));
    let done = match action {
        "delete" => actions::delete_account(console, argument).await,
        "kickall" => actions::kick_all(console, argument == "1").await,
        "maintkick" => actions::set_maintenance(console, true, true).await,
        "shutdown" => actions::shutdown(console).await,
        _ => return,
    };
    let notice = bot.record(http, &interaction.user, done).await;
    log_failure(
        interaction
            .edit_response(http, edit(views::notice(&notice)))
            .await,
        rest,
    );
}

fn modal_values(data: &ModalInteractionData) -> HashMap<String, String> {
    data.components
        .iter()
        .filter_map(|component| match component {
            ModalComponent::Label(label) => match &label.component {
                LabelComponent::InputText(input) => {
                    Some((input.custom_id.to_string(), input.value.to_string()))
                }
                _ => None,
            },
            _ => None,
        })
        .collect()
}

pub async fn modal(bot: &Bot, ctx: &Context, interaction: &ModalInteraction) {
    let http = &ctx.http;
    let console = &bot.console_for(&interaction.user);
    let from_card = interaction.message.is_some();
    let deferred = if from_card {
        interaction.defer(http).await
    } else {
        interaction.defer_ephemeral(http).await
    };
    if deferred.is_err() {
        return;
    }
    let values = modal_values(&interaction.data);
    let value = |key: &str| {
        values
            .get(key)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
    };
    let id = interaction.data.custom_id.as_str();
    let Some((kind, rest)) = id.strip_prefix("modal:").and_then(|id| id.split_once(':')) else {
        return;
    };
    let password_check = || -> Result<String, String> {
        let password = values.get("password").cloned().unwrap_or_default();
        if values.get("confirm") != Some(&password) {
            return Err("비밀번호가 서로 다릅니다".to_owned());
        }
        Ok(password)
    };
    let (nickname, done) = match kind {
        "create" => {
            let (admin, nickname) = rest.split_once(':').unwrap_or(("0", rest));
            let done = match password_check() {
                Ok(password) => {
                    actions::create_account(console, nickname, &password, admin == "1").await
                }
                Err(error) => Action::refused(format!("계정 생성 {}", escape(nickname)), error),
            };
            (nickname, done)
        }
        "pw" => {
            let done = match password_check() {
                Ok(password) => actions::reset_password(console, rest, &password).await,
                Err(error) => Action::refused(format!("비밀번호 변경 {}", escape(rest)), error),
            };
            (rest, done)
        }
        "ban" => (
            rest,
            actions::ban(console, rest, value("reason"), value("duration")).await,
        ),
        _ => return,
    };
    let notice = bot.record(http, &interaction.user, done).await;
    let components = if from_card {
        account_card(console, nickname, Some(&notice)).await
    } else {
        views::notice(&notice)
    };
    log_failure(interaction.edit_response(http, edit(components)).await, id);
}
