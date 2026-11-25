use std::{collections::BTreeSet, time::Instant};

use serenity::{
    builder::{
        AutocompleteChoice, CreateAutocompleteResponse, CreateCommand, CreateCommandOption,
        CreateInteractionResponse, GetMessages,
    },
    model::{
        application::{
            CommandData, CommandInteraction, CommandOptionType, ResolvedOption, ResolvedValue,
        },
        id::MessageId,
        permissions::Permissions,
    },
    prelude::Context,
};

use crate::{
    actions::{self, Action},
    bot::{Bot, OPERATOR_PERMISSIONS, PUBLIC_COMMANDS, edit, log_failure, new_message},
    components,
    console::ConsoleResult,
    format::mode_label,
    monitor::{Snapshot, unix_now},
    views::{self, AccountFilter, Components, Tone},
};

const MAX_CHOICES: usize = 25;
const BULK_DELETE_WINDOW: u64 = 14 * 24 * 60 * 60;

fn sub(name: &'static str, description: &'static str) -> CreateCommandOption<'static> {
    CreateCommandOption::new(CommandOptionType::SubCommand, name, description)
}

fn text(
    name: &'static str,
    description: &'static str,
    required: bool,
) -> CreateCommandOption<'static> {
    CreateCommandOption::new(CommandOptionType::String, name, description).required(required)
}

fn suggested(
    name: &'static str,
    description: &'static str,
    required: bool,
) -> CreateCommandOption<'static> {
    text(name, description, required).set_autocomplete(true)
}

fn flag(
    name: &'static str,
    description: &'static str,
    required: bool,
) -> CreateCommandOption<'static> {
    CreateCommandOption::new(CommandOptionType::Boolean, name, description).required(required)
}

fn number(
    name: &'static str,
    description: &'static str,
    required: bool,
) -> CreateCommandOption<'static> {
    CreateCommandOption::new(CommandOptionType::Integer, name, description)
        .required(required)
        .min_int_value(0)
}

fn nickname(description: &'static str) -> CreateCommandOption<'static> {
    suggested("nickname", description, true).max_length(64)
}

#[allow(clippy::too_many_lines, reason = "one block per command")]
pub fn definitions() -> Vec<CreateCommand<'static>> {
    let command = |name: &'static str, description: &'static str| {
        let command = CreateCommand::new(name).description(description);
        if PUBLIC_COMMANDS.contains(&name) {
            command
        } else {
            command.default_member_permissions(OPERATOR_PERMISSIONS)
        }
    };
    vec![
        command("status", "게임 서버 상태 요약을 봅니다"),
        command("online", "지금 접속한 라이더를 봅니다"),
        command("rooms", "열린 방과 참가자를 봅니다"),
        command("ping", "봇과 운영 콘솔의 응답 속도를 잽니다"),
        command("help", "명령어 도움말을 봅니다"),
        command(
            "settings",
            "알림·기록 채널, 접속 알림, 갱신 간격을 설정합니다",
        ),
        command("dashboard", "자동으로 갱신되는 서버 대시보드")
            .add_option(sub("create", "이 채널에 대시보드를 올립니다"))
            .add_option(sub("remove", "이 채널의 대시보드를 지웁니다")),
        command("panel", "버튼으로 조작하는 운영 패널을 엽니다"),
        command("players", "접속자 상세(회원 번호·IP·접속 시간)를 봅니다"),
        command("account", "계정 관리")
            .add_option(
                sub("list", "계정 목록을 봅니다").add_sub_option(
                    text("filter", "보여 줄 계정", false)
                        .add_string_choice("전체", "all")
                        .add_string_choice("관리자", "admin")
                        .add_string_choice("정지", "banned"),
                ),
            )
            .add_option(
                sub("info", "계정 상세와 빠른 조작 버튼").add_sub_option(nickname("닉네임")),
            )
            .add_option(
                sub("create", "계정을 만듭니다(가입이 닫혀 있어도 가능)")
                    .add_sub_option(text("nickname", "닉네임", true).max_length(64))
                    .add_sub_option(flag("admin", "관리자 계정으로 만들기", false)),
            )
            .add_option(
                sub("delete", "계정을 삭제하고 퇴장시킵니다").add_sub_option(nickname("닉네임")),
            )
            .add_option(
                sub("password", "비밀번호를 재설정합니다").add_sub_option(nickname("닉네임")),
            )
            .add_option(
                sub("admin", "관리자(옵저버·익명 리그 역할) 지정/해제")
                    .add_sub_option(nickname("닉네임"))
                    .add_sub_option(flag("enabled", "관리자로 지정할지", true)),
            ),
        command("ban", "정지·차단")
            .add_option(
                sub("account", "계정을 정지하고 퇴장시킵니다")
                    .add_sub_option(nickname("닉네임"))
                    .add_sub_option(text("reason", "사유", false).max_length(200))
                    .add_sub_option(text(
                        "duration",
                        "기간(예: 30m, 12h, 7d, 2주). 비우면 무기한",
                        false,
                    )),
            )
            .add_option(
                sub("ip", "IP를 차단하고 그 주소의 연결을 끊습니다")
                    .add_sub_option(suggested("address", "IP 주소", true))
                    .add_sub_option(text("reason", "사유", false).max_length(200)),
            )
            .add_option(sub("list", "차단한 IP 목록을 봅니다")),
        command("unban", "정지·차단 해제")
            .add_option(sub("account", "계정 정지를 풉니다").add_sub_option(nickname("닉네임")))
            .add_option(sub("ip", "IP 차단을 풉니다").add_sub_option(suggested(
                "address",
                "IP 주소",
                true,
            ))),
        command("kick", "강제 퇴장")
            .add_option(
                sub("player", "라이더 한 명을 퇴장시킵니다")
                    .add_sub_option(nickname("접속 중인 닉네임")),
            )
            .add_option(
                sub("ip", "한 주소의 모든 연결을 끊습니다(로그인 전 연결 포함)")
                    .add_sub_option(suggested("address", "IP 주소", true)),
            )
            .add_option(
                sub("room", "방 안의 라이더·옵저버를 모두 퇴장시킵니다")
                    .add_sub_option(number("id", "방 번호", true).set_autocomplete(true)),
            )
            .add_option(sub("all", "전원 퇴장시킵니다").add_sub_option(flag(
                "keep_admins",
                "관리자 계정은 남기기",
                false,
            ))),
        command("maintenance", "점검 모드(재시작하면 꺼짐)")
            .add_option(sub("status", "점검 모드 상태를 봅니다"))
            .add_option(
                sub("on", "점검 모드를 켭니다. 관리자만 로그인할 수 있습니다")
                    .add_sub_option(flag("kick", "관리자 외 전원 퇴장", false)),
            )
            .add_option(sub("off", "점검 모드를 끕니다")),
        command(
            "registration",
            "실행기 가입을 열거나 닫습니다(재시작 전까지)",
        )
        .add_option(
            text("state", "가입 상태", true)
                .add_string_choice("열기", "open")
                .add_string_choice("닫기", "close"),
        ),
        command("rewards", "경기 보상 저장")
            .add_option(sub("list", "저장 중인 보상과 저장 실패를 봅니다"))
            .add_option(
                sub(
                    "retry",
                    "저장 실패를 재시도합니다(비우면 재시도 가능한 것 전부)",
                )
                .add_sub_option(number("room", "방 번호", false).set_autocomplete(true))
                .add_sub_option(number("race_epoch", "경기 번호", false).set_autocomplete(true)),
            ),
        command("throttles", "로그인·콘솔 시도 제한")
            .add_option(sub("list", "제한에 걸렸거나 가까운 주소를 봅니다"))
            .add_option(
                sub("clear", "시도 기록을 지웁니다(비우면 전부)").add_sub_option(suggested(
                    "address",
                    "IP 주소",
                    false,
                )),
            ),
        command("audit", "서버 재시작 이후 운영 기록을 봅니다").add_option(
            number("limit", "건수(기본 20)", false)
                .min_int_value(1)
                .max_int_value(256),
        ),
        command("shutdown", "서버를 정상 종료합니다"),
        CreateCommand::new("purge")
            .description("이 채널의 최근 메시지를 지웁니다(대시보드 제외)")
            .default_member_permissions(Permissions::MANAGE_MESSAGES)
            .add_option(
                number("count", "지울 메시지 수", true)
                    .min_int_value(1)
                    .max_int_value(100),
            ),
    ]
}

struct Invocation<'a> {
    path: Vec<&'a str>,
    options: Vec<ResolvedOption<'a>>,
}

impl<'a> Invocation<'a> {
    fn new(data: &'a CommandData) -> Self {
        let mut path = Vec::new();
        let mut options = data.options();
        while let [
            ResolvedOption {
                value: ResolvedValue::SubCommand(_) | ResolvedValue::SubCommandGroup(_),
                ..
            },
        ] = options.as_slice()
        {
            let option = options.remove(0);
            path.push(option.name);
            if let ResolvedValue::SubCommand(inner) | ResolvedValue::SubCommandGroup(inner) =
                option.value
            {
                options = inner.into_iter().collect();
            }
        }
        Self { path, options }
    }

    fn value(&self, name: &str) -> Option<&ResolvedValue<'a>> {
        self.options
            .iter()
            .find(|option| option.name == name)
            .map(|option| &option.value)
    }

    fn string(&self, name: &str) -> Option<&'a str> {
        match self.value(name)? {
            ResolvedValue::String(text) => Some(text.trim()).filter(|text| !text.is_empty()),
            _ => None,
        }
    }

    fn integer(&self, name: &str) -> Option<i64> {
        match self.value(name)? {
            ResolvedValue::Integer(value) => Some(*value),
            ResolvedValue::Autocomplete { value, .. } => value.trim().parse().ok(),
            _ => None,
        }
    }

    fn boolean(&self, name: &str) -> Option<bool> {
        match self.value(name)? {
            ResolvedValue::Boolean(value) => Some(*value),
            _ => None,
        }
    }
}

pub async fn run(bot: &Bot, ctx: &Context, command: &CommandInteraction) {
    let http = &ctx.http;
    let name = command.data.name.as_str();
    let args = Invocation::new(&command.data);
    if let Some(response) = immediate(bot, name, &args) {
        log_failure(command.create_response(http, response).await, name);
        return;
    }

    let ephemeral = !matches!(name, "status" | "online" | "rooms" | "ping");
    let started = Instant::now();
    let deferred = if ephemeral {
        command.defer_ephemeral(http).await
    } else {
        command.defer(http).await
    };
    if let Err(error) = deferred {
        tracing::warn!(%error, name, "failed to defer a command");
        return;
    }
    let discord_ms = started.elapsed().as_millis();
    let components = execute(bot, ctx, command, name, &args, discord_ms).await;
    log_failure(command.edit_response(http, edit(components)).await, name);
}

fn immediate(
    bot: &Bot,
    name: &str,
    args: &Invocation<'_>,
) -> Option<CreateInteractionResponse<'static>> {
    let nickname = args.string("nickname").unwrap_or_default();
    Some(match (name, args.path.as_slice()) {
        ("help", _) => new_message(views::help(), true),
        ("settings", _) => new_message(views::settings(&bot.monitor.settings(), None), true),
        ("account", ["create"]) => CreateInteractionResponse::Modal(components::password_modal(
            format!(
                "modal:create:{}:{nickname}",
                u8::from(args.boolean("admin").unwrap_or(false))
            ),
            "계정 생성",
            nickname,
        )),
        ("account", ["password"]) => CreateInteractionResponse::Modal(components::password_modal(
            format!("modal:pw:{nickname}"),
            "비밀번호 변경",
            nickname,
        )),
        ("account", ["delete"]) => new_message(components::delete_confirm(nickname), true),
        ("kick", ["all"]) => {
            let keep_admins = args.boolean("keep_admins").unwrap_or(false);
            new_message(components::kick_all_confirm(keep_admins), true)
        }
        ("maintenance", ["on"]) if args.boolean("kick") == Some(true) => new_message(
            views::confirm("점검을 켜고 관리자 외 퇴장할까요?", "maintkick"),
            true,
        ),
        ("shutdown", _) => new_message(components::shutdown_confirm(), true),
        _ => return None,
    })
}

fn show<T>(result: ConsoleResult<T>, view: impl FnOnce(T) -> Components) -> Components {
    result.map_or_else(|error| views::error(error.to_string()), view)
}

#[allow(clippy::too_many_lines, reason = "one arm per command")]
async fn execute(
    bot: &Bot,
    ctx: &Context,
    command: &CommandInteraction,
    name: &str,
    args: &Invocation<'_>,
    discord_ms: u128,
) -> Components {
    let http = &ctx.http;
    let console = &bot.console_for(&command.user);
    let user = &command.user;
    let nickname = args.string("nickname").unwrap_or_default();
    let address = args.string("address").unwrap_or_default();
    let action = match (name, args.path.as_slice()) {
        ("status", _) => {
            return match Snapshot::fetch(console).await {
                Ok(snapshot) => views::status(snapshot),
                Err(error) => views::offline(&error.to_string()),
            };
        }
        ("online", _) => {
            return match tokio::try_join!(console.players(), console.rooms()) {
                Ok((players, rooms)) => views::online(&players, &rooms),
                Err(error) => views::offline(&error.to_string()),
            };
        }
        ("rooms", _) => {
            return match console.rooms().await {
                Ok(rooms) => views::rooms(&rooms),
                Err(error) => views::offline(&error.to_string()),
            };
        }
        ("ping", _) => {
            let started = Instant::now();
            let reply = console.status().await;
            let console_ms = started.elapsed().as_millis();
            let console = match reply {
                Ok(_) => format!("{console_ms}ms"),
                Err(error) => error.to_string(),
            };
            return views::message(
                Tone::Info,
                format!("Discord {discord_ms}ms · 서버 {console}"),
            );
        }
        ("dashboard", ["create"]) => {
            let label = "대시보드 만들기";
            match bot.monitor.create_dashboard(http, command.channel_id).await {
                Ok(thread) => Action {
                    label: label.to_owned(),
                    outcome: Ok(match thread {
                        Some(thread) => format!("대시보드 생성 · 알림 채널 <#{thread}>"),
                        None => "대시보드 생성 · 알림 스레드를 만들 수 없음".to_owned(),
                    }),
                },
                Err(error) => Action::refused(
                    label,
                    format!("이 채널에 메시지를 보낼 수 없습니다 ({error})"),
                ),
            }
        }
        ("dashboard", ["remove"]) => {
            let removed = bot
                .monitor
                .remove_dashboards(http, command.channel_id)
                .await;
            if removed == 0 {
                return views::message(Tone::Info, "대시보드 없음");
            }
            Action {
                label: "대시보드 제거".to_owned(),
                outcome: Ok(format!("대시보드 {removed}개 삭제")),
            }
        }
        ("panel", _) => return components::panel(console, None).await,
        ("players", _) => {
            return show(
                tokio::try_join!(console.players(), console.rooms()),
                |(players, rooms)| views::players(&players, &rooms),
            );
        }
        ("account", ["list"]) => {
            let filter = AccountFilter::parse(args.string("filter").unwrap_or("all"));
            return show(console.accounts().await, |accounts| {
                views::accounts(&accounts, filter, 0)
            });
        }
        ("account", ["info"]) => return components::account_card(console, nickname, None).await,
        ("account", ["admin"]) => {
            actions::set_admin(console, nickname, args.boolean("enabled").unwrap_or(false)).await
        }
        ("ban", ["account"]) => {
            actions::ban(
                console,
                nickname,
                args.string("reason"),
                args.string("duration"),
            )
            .await
        }
        ("ban", ["ip"]) => actions::ban_address(console, address, args.string("reason")).await,
        ("ban", ["list"]) => {
            return show(console.address_bans().await, |bans| {
                views::address_bans(&bans)
            });
        }
        ("unban", ["account"]) => actions::unban(console, nickname).await,
        ("unban", ["ip"]) => actions::unban_address(console, address).await,
        ("kick", ["player"]) => actions::kick(console, nickname).await,
        ("kick", ["ip"]) => actions::kick_address(console, address).await,
        ("kick", ["room"]) => match args.integer("id").and_then(|id| u32::try_from(id).ok()) {
            Some(id) => actions::kick_room(console, id).await,
            None => Action::refused("방 퇴장", "잘못된 방 번호"),
        },
        ("maintenance", ["status"]) => {
            return show(console.status().await, |status| {
                if status.maintenance {
                    views::message(Tone::Warning, "점검 모드 켜짐")
                } else {
                    views::message(Tone::Success, "점검 모드 꺼짐")
                }
            });
        }
        ("maintenance", ["on"]) => actions::set_maintenance(console, true, false).await,
        ("maintenance", ["off"]) => actions::set_maintenance(console, false, false).await,
        ("registration", _) => {
            actions::set_registration(console, args.string("state") == Some("open")).await
        }
        ("rewards", ["list"]) => {
            return show(console.rewards().await, |rewards| views::rewards(&rewards));
        }
        ("rewards", ["retry"]) => {
            let room = args
                .integer("room")
                .and_then(|room| u32::try_from(room).ok());
            let epoch = args
                .integer("race_epoch")
                .and_then(|epoch| u64::try_from(epoch).ok());
            match (room, epoch) {
                (Some(room), Some(epoch)) => {
                    actions::retry_rewards(console, Some((room, epoch))).await
                }
                (None, None) => actions::retry_rewards(console, None).await,
                _ => Action::refused(
                    "보상 재시도",
                    "방 번호와 경기 번호를 함께 적거나 둘 다 비우세요",
                ),
            }
        }
        ("throttles", ["list"]) => {
            return show(console.throttles().await, |throttles| {
                views::throttles(&throttles)
            });
        }
        ("throttles", ["clear"]) => actions::clear_throttles(console, args.string("address")).await,
        ("audit", _) => {
            let limit = args
                .integer("limit")
                .and_then(|limit| usize::try_from(limit).ok())
                .unwrap_or(20);
            return show(console.audit(limit).await, |entries| views::audit(&entries));
        }
        ("purge", _) => purge(bot, ctx, command, args.integer("count").unwrap_or(10)).await,
        _ => return views::error("알 수 없는 명령"),
    };
    views::notice(&bot.record(http, user, action).await)
}

async fn purge(bot: &Bot, ctx: &Context, command: &CommandInteraction, count: i64) -> Action {
    let http = &ctx.http;
    let channel = command.channel_id;
    let label = format!("메시지 정리 <#{channel}>");
    let limit = u8::try_from(count.clamp(1, 100)).unwrap_or(100);
    let messages = match channel
        .messages(http, GetMessages::new().limit(limit))
        .await
    {
        Ok(messages) => messages,
        Err(error) => {
            return Action::refused(label, format!("메시지를 읽을 수 없습니다 ({error})"));
        }
    };
    let dashboards = bot.monitor.dashboard_messages().await;
    let cutoff = unix_now().saturating_sub(BULK_DELETE_WINDOW) + 60;
    let ids: Vec<MessageId> = messages
        .iter()
        .filter(|message| !dashboards.contains(&message.id))
        .filter(|message| {
            u64::try_from(message.id.created_at().unix_timestamp()).unwrap_or(0) > cutoff
        })
        .map(|message| message.id)
        .collect();
    let deleted = match ids.as_slice() {
        [] => {
            return Action::refused(label, "지울 메시지 없음 (2주 넘은 메시지 제외)");
        }
        [id] => channel.delete_message(http, *id, Some("/purge")).await,
        ids => channel.delete_messages(http, ids, Some("/purge")).await,
    };
    match deleted {
        Ok(()) => Action {
            label,
            outcome: Ok(format!("메시지 {}개 삭제", ids.len())),
        },
        Err(error) => Action::refused(label, format!("삭제할 수 없습니다 ({error})")),
    }
}

fn choice(
    name: String,
    value: impl Into<serenity::builder::AutocompleteValue<'static>>,
) -> AutocompleteChoice<'static> {
    let name = if name.chars().count() > 100 {
        name.chars().take(99).collect::<String>() + "…"
    } else {
        name
    };
    AutocompleteChoice::new(name, value)
}

pub async fn autocomplete(bot: &Bot, ctx: &Context, command: &CommandInteraction) {
    let mut choices = Vec::new();
    if let Some(focused) = command.data.autocomplete() {
        let args = Invocation::new(&command.data);
        choices = suggestions(
            bot,
            command.data.name.as_str(),
            &args,
            focused.name,
            focused.value,
        )
        .await;
    }
    let response = CreateInteractionResponse::Autocomplete(
        CreateAutocompleteResponse::new().set_choices(choices),
    );
    log_failure(
        command.create_response(&ctx.http, response).await,
        "autocomplete",
    );
}

#[allow(clippy::too_many_lines, reason = "one arm per option")]
async fn suggestions(
    bot: &Bot,
    name: &str,
    args: &Invocation<'_>,
    option: &str,
    typed: &str,
) -> Vec<AutocompleteChoice<'static>> {
    let console = bot.console();
    let typed = typed.trim().to_lowercase();
    let wanted = |text: &str| text.to_lowercase().contains(&typed);
    let path = args.path.as_slice();
    let mut choices: Vec<AutocompleteChoice<'static>> = match (name, path, option) {
        ("kick", ["player"], "nickname") => {
            let Ok(players) = console.players().await else {
                return Vec::new();
            };
            players
                .into_iter()
                .filter(|player| wanted(&player.nickname))
                .map(|player| {
                    let place = player
                        .room
                        .map_or_else(|| "로비".to_owned(), |room| format!("방 #{room}"));
                    choice(format!("{} · {place}", player.nickname), player.nickname)
                })
                .collect()
        }
        (_, _, "nickname") => {
            let Ok(accounts) = console.accounts().await else {
                return Vec::new();
            };
            let only_banned = name == "unban";
            accounts
                .into_iter()
                .filter(|account| !only_banned || account.banned)
                .filter(|account| wanted(&account.nickname))
                .map(|account| {
                    let mut label = account.nickname.clone();
                    if account.admin {
                        label.push_str(" · 관리자");
                    }
                    if account.banned {
                        label.push_str(" · 정지");
                    }
                    choice(label, account.nickname)
                })
                .collect()
        }
        ("kick" | "ban", ["ip"], "address") => {
            let Ok(players) = console.players().await else {
                return Vec::new();
            };
            let mut seen = std::collections::BTreeMap::<String, Vec<String>>::new();
            for player in players {
                seen.entry(player.address)
                    .or_default()
                    .push(player.nickname);
            }
            seen.into_iter()
                .filter(|(address, names)| wanted(address) || names.iter().any(|name| wanted(name)))
                .map(|(address, names)| {
                    choice(format!("{address} · {}", names.join(", ")), address)
                })
                .collect()
        }
        ("unban", ["ip"], "address") => {
            let Ok(bans) = console.address_bans().await else {
                return Vec::new();
            };
            bans.into_iter()
                .filter(|ban| wanted(&ban.address))
                .map(|ban| {
                    let label = match &ban.reason {
                        Some(reason) => format!("{} · {reason}", ban.address),
                        None => ban.address.clone(),
                    };
                    choice(label, ban.address)
                })
                .collect()
        }
        ("throttles", ["clear"], "address") => {
            let Ok(throttles) = console.throttles().await else {
                return Vec::new();
            };
            let mut addresses: Vec<(String, bool)> = throttles
                .launcher
                .into_iter()
                .map(|entry| (entry.address, entry.limited))
                .chain(
                    throttles
                        .console
                        .into_iter()
                        .map(|entry| (entry.address, entry.limited)),
                )
                .collect();
            addresses.sort();
            addresses.dedup_by(|left, right| left.0 == right.0);
            addresses
                .into_iter()
                .filter(|(address, _)| wanted(address))
                .map(|(address, limited)| {
                    choice(
                        format!("{address}{}", if limited { " · 제한됨" } else { "" }),
                        address,
                    )
                })
                .collect()
        }
        ("kick", ["room"], "id") => {
            let Ok(rooms) = console.rooms().await else {
                return Vec::new();
            };
            rooms
                .into_iter()
                .filter(|room| wanted(&room.id.to_string()) || wanted(&room.name))
                .map(|room| {
                    choice(
                        format!(
                            "#{} {} · {} · {}/{}명",
                            room.id,
                            room.name,
                            mode_label(room.game_type, room.speed_type),
                            room.players.len(),
                            room.capacity
                        ),
                        u64::from(room.id),
                    )
                })
                .collect()
        }
        ("rewards", ["retry"], "room" | "race_epoch") => {
            let Ok(rewards) = console.rewards().await else {
                return Vec::new();
            };
            let room_filter = if option == "race_epoch" {
                args.integer("room")
            } else {
                None
            };
            let mut seen = BTreeSet::new();
            rewards
                .dead_letters
                .into_iter()
                .filter(|dead| dead.retryable)
                .filter(|dead| option != "room" || seen.insert(dead.room))
                .filter(|dead| room_filter.is_none_or(|room| i64::from(dead.room) == room))
                .filter(|dead| {
                    wanted(&if option == "room" {
                        dead.room.to_string()
                    } else {
                        dead.race_epoch.to_string()
                    })
                })
                .map(|dead| {
                    let label = format!(
                        "방 #{} · 경기 {} · {}",
                        dead.room,
                        dead.race_epoch,
                        dead.nickname.as_deref().unwrap_or("-")
                    );
                    let value = if option == "room" {
                        u64::from(dead.room)
                    } else {
                        dead.race_epoch
                    };
                    choice(label, value)
                })
                .collect()
        }
        _ => Vec::new(),
    };
    choices.truncate(MAX_CHOICES);
    choices
}
