use std::{collections::BTreeMap, fmt::Write as _, sync::Arc};

use serenity::{
    builder::{
        CreateActionRow, CreateButton, CreateComponent, CreateContainer, CreateContainerComponent,
        CreateSection, CreateSectionAccessory, CreateSectionComponent, CreateSelectMenu,
        CreateSelectMenuKind, CreateSelectMenuOption, CreateSeparator, CreateTextDisplay,
    },
    model::{Colour, application::ButtonStyle, channel::ChannelType, id::GenericChannelId},
};

use crate::{
    console::{
        Account, AccountInfo, AddressBan, AuditEntry, Player, Rewards, Room, Status, Throttles,
    },
    format::{code, escape, fit_lines, mode_label, on_off, open_closed, phase_label, timestamp},
    monitor::{Health, POLL_CHOICES, Settings, Snapshot},
};

pub type Components = Vec<CreateComponent<'static>>;

const GREEN: u32 = 0x2E_CC_71;
const ORANGE: u32 = 0xF3_9C_12;
const RED: u32 = 0xE7_4C_3C;
const BLURPLE: u32 = 0x58_65_F2;
const GREY: u32 = 0x95_A5_A6;

const ACCOUNTS_PER_PAGE: usize = 20;

struct Card {
    accent: u32,
    parts: Vec<CreateContainerComponent<'static>>,
}

impl Card {
    fn new(accent: u32) -> Self {
        Self {
            accent,
            parts: Vec::new(),
        }
    }

    #[must_use]
    fn text(mut self, content: impl Into<String>) -> Self {
        let content = content.into();
        if !content.trim().is_empty() {
            self.parts.push(CreateContainerComponent::TextDisplay(
                CreateTextDisplay::new(content),
            ));
        }
        self
    }

    #[must_use]
    fn divider(mut self) -> Self {
        self.parts.push(CreateContainerComponent::Separator(
            CreateSeparator::new().divider(true),
        ));
        self
    }

    #[must_use]
    fn section(mut self, content: impl Into<String>, button: CreateButton<'static>) -> Self {
        self.parts
            .push(CreateContainerComponent::Section(CreateSection::new(
                vec![CreateSectionComponent::TextDisplay(CreateTextDisplay::new(
                    content.into(),
                ))],
                CreateSectionAccessory::Button(button),
            )));
        self
    }

    #[must_use]
    fn buttons(mut self, buttons: Vec<CreateButton<'static>>) -> Self {
        if !buttons.is_empty() {
            self.parts.push(CreateContainerComponent::ActionRow(
                CreateActionRow::buttons(buttons),
            ));
        }
        self
    }

    #[must_use]
    fn select(mut self, menu: CreateSelectMenu<'static>) -> Self {
        self.parts.push(CreateContainerComponent::ActionRow(
            CreateActionRow::select_menu(menu),
        ));
        self
    }

    fn build(self) -> Components {
        vec![CreateComponent::Container(
            CreateContainer::new(self.parts).accent_colour(Colour::new(self.accent)),
        )]
    }
}

fn button(id: impl Into<String>, label: &str, style: ButtonStyle) -> CreateButton<'static> {
    CreateButton::new(id.into())
        .label(label.to_owned())
        .style(style)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Success,
    Info,
    Warning,
    Error,
}

impl Tone {
    const fn colour(self) -> u32 {
        match self {
            Self::Success => GREEN,
            Self::Info => BLURPLE,
            Self::Warning => ORANGE,
            Self::Error => RED,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Notice {
    pub tone: Tone,
    pub text: String,
}

impl Notice {
    pub fn new(tone: Tone, text: impl Into<String>) -> Self {
        Self {
            tone,
            text: text.into(),
        }
    }

    fn line(&self) -> String {
        match self.tone {
            Tone::Error => format!("실패: {}", self.text),
            _ => self.text.clone(),
        }
    }
}

pub fn message(tone: Tone, text: impl Into<String>) -> Components {
    Card::new(tone.colour()).text(text).build()
}

pub fn notice(notice: &Notice) -> Components {
    message(notice.tone, notice.text.clone())
}

pub fn error(text: impl Into<String>) -> Components {
    message(Tone::Error, text)
}

pub fn confirm(question: &str, action: &str) -> Components {
    Card::new(ORANGE)
        .text(format!("**{question}**"))
        .buttons(vec![
            button(format!("cfm:{action}"), "실행", ButtonStyle::Danger),
            button("cfm:cancel", "취소", ButtonStyle::Secondary),
        ])
        .build()
}

fn status_colour(status: &Status) -> u32 {
    if status.maintenance { ORANGE } else { GREEN }
}

fn header(card: Card, health: &Health) -> Card {
    let card = card.text("## 카트라이더 P5099");
    match health {
        Health::Starting => card.text("확인 중"),
        Health::Offline { since, error } => card.text(format!(
            "**응답 없음** · {}부터\n-# {}",
            timestamp(*since, 'R'),
            escape(error)
        )),
        Health::Online(snapshot) => {
            let status = &snapshot.status;
            let mut line = format!(
                "**{}** · v{} · {} 시작",
                if status.maintenance {
                    "점검 중"
                } else {
                    "정상"
                },
                escape(&status.version),
                timestamp(snapshot.started, 'R'),
            );
            if status.dead_letters() > 0 {
                let _ = write!(line, "\n보상 저장 실패 {}건", status.dead_letters());
            }
            card.text(line)
        }
    }
}

fn stats(card: Card, status: &Status) -> Card {
    let mut line = format!(
        "접속자 {} · 방 {} · 경기 중 {} · 계정 {}",
        status.players_online, status.rooms, status.racing_rooms, status.accounts,
    );
    if status.require_account && !status.registration_open {
        line.push_str(" · 가입 닫힘");
    }
    card.text(line)
}

fn room_lines(rooms: &[Room]) -> Vec<String> {
    let mut rooms: Vec<&Room> = rooms.iter().collect();
    rooms.sort_by_key(|room| room.id);
    rooms
        .into_iter()
        .map(|room| {
            let riders = room
                .players
                .iter()
                .map(|name| escape(name))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "**#{}** {}{}\n-# {} · {} · 트랙 {} · {}/{}{}",
                room.id,
                room_name(room),
                if room.locked { " (잠김)" } else { "" },
                phase_label(&room.phase),
                mode_label(room.game_type, room.speed_type),
                room.track,
                room.players.len(),
                room.capacity,
                if riders.is_empty() {
                    String::new()
                } else {
                    format!("\n{riders}")
                },
            )
        })
        .collect()
}

fn room_name(room: &Room) -> String {
    if room.name.trim().is_empty() {
        "(이름 없음)".to_owned()
    } else {
        escape(&room.name)
    }
}

fn location(player: &Player, rooms: &[Room]) -> String {
    match player.room {
        Some(id) => match rooms.iter().find(|room| room.id == id) {
            Some(room) => format!("#{id} {}", room_name(room)),
            None => format!("#{id}"),
        },
        None => "로비".to_owned(),
    }
}

fn sorted_players(players: &[Player]) -> Vec<&Player> {
    let mut sorted: Vec<&Player> = players.iter().collect();
    sorted.sort_by(|left, right| left.nickname.cmp(&right.nickname));
    sorted
}

pub fn dashboard(health: &Health, changed: u64) -> Components {
    let accent = match health {
        Health::Starting => GREY,
        Health::Offline { .. } => RED,
        Health::Online(snapshot) => status_colour(&snapshot.status),
    };
    let mut card = header(Card::new(accent), health);
    if let Health::Online(snapshot) = health {
        let players: Vec<String> = sorted_players(&snapshot.players)
            .into_iter()
            .map(|player| {
                format!(
                    "**{}** {}",
                    escape(&player.nickname),
                    location(player, &snapshot.rooms)
                )
            })
            .collect();
        card = stats(card, &snapshot.status)
            .divider()
            .text(format!("### 방 {}", snapshot.rooms.len()))
            .text(fit_lines(&room_lines(&snapshot.rooms), 1_500, "없음"))
            .divider()
            .text(format!("### 접속자 {}", snapshot.players.len()))
            .text(fit_lines(&players, 1_200, "없음"));
    }
    card.divider()
        .text(format!("-# {} 갱신", timestamp(changed, 'R')))
        .buttons(vec![
            button("dash:refresh", "새로고침", ButtonStyle::Secondary),
            button("dash:players", "접속자", ButtonStyle::Secondary),
            button("dash:rooms", "방", ButtonStyle::Secondary),
            button("dash:panel", "운영 패널", ButtonStyle::Primary),
        ])
        .build()
}

pub fn status(snapshot: Snapshot) -> Components {
    let card = Card::new(status_colour(&snapshot.status));
    let status = snapshot.status.clone();
    stats(header(card, &Health::Online(Arc::new(snapshot))), &status).build()
}

pub fn offline(error: &str) -> Components {
    Card::new(RED)
        .text("## 카트라이더 P5099")
        .text(format!("**응답 없음**\n-# {}", escape(error)))
        .build()
}

pub fn online(players: &[Player], rooms: &[Room]) -> Components {
    let mut places: BTreeMap<(u8, u32), (String, Vec<String>)> = BTreeMap::new();
    for player in sorted_players(players) {
        let key = player.room.map_or((0, 0), |id| (1, id));
        places
            .entry(key)
            .or_insert_with(|| (location(player, rooms), Vec::new()))
            .1
            .push(escape(&player.nickname));
    }
    let lines: Vec<String> = places
        .into_values()
        .map(|(place, names)| format!("**{place}**\n{}", names.join(", ")))
        .collect();
    Card::new(GREEN)
        .text(format!("### 접속자 {}", players.len()))
        .text(fit_lines(&lines, 3_500, "없음"))
        .build()
}

pub fn rooms(rooms: &[Room]) -> Components {
    let mut sorted: Vec<&Room> = rooms.iter().collect();
    sorted.sort_by_key(|room| room.id);
    let lines: Vec<String> = room_lines(rooms)
        .into_iter()
        .zip(sorted)
        .map(|(line, room)| {
            if room.observers.is_empty() {
                line
            } else {
                let observers = room
                    .observers
                    .iter()
                    .map(|name| escape(name))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{line}\n-# 옵저버 {observers}")
            }
        })
        .collect();
    Card::new(BLURPLE)
        .text(format!("### 방 {}", rooms.len()))
        .text(fit_lines(&lines, 3_500, "없음"))
        .build()
}

pub fn players(players: &[Player], rooms: &[Room]) -> Components {
    let lines: Vec<String> = sorted_players(players)
        .into_iter()
        .map(|player| {
            format!(
                "**{}**{} #{} · {} · {}",
                escape(&player.nickname),
                if player.admin { " (관리자)" } else { "" },
                player.user_no,
                code(&player.address),
                location(player, rooms),
            )
        })
        .collect();
    Card::new(BLURPLE)
        .text(format!("### 접속자 {}", players.len()))
        .text(fit_lines(&lines, 3_500, "없음"))
        .build()
}

fn account_flags(account: &Account) -> String {
    let mut flags = Vec::new();
    if account.admin {
        flags.push("관리자".to_owned());
    }
    if account.banned {
        let mut flag = "정지".to_owned();
        if let Some(reason) = &account.ban_reason {
            let _ = write!(flag, " ({})", escape(reason));
        }
        if let Some(until) = account.banned_until {
            let _ = write!(flag, " ~{}", timestamp(until, 'f'));
        }
        flags.push(flag);
    }
    flags.join(" · ")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountFilter {
    All,
    Admins,
    Banned,
}

impl AccountFilter {
    pub fn parse(text: &str) -> Self {
        match text {
            "admin" => Self::Admins,
            "banned" => Self::Banned,
            _ => Self::All,
        }
    }

    pub const fn key(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Admins => "admin",
            Self::Banned => "banned",
        }
    }

    const fn title(self) -> &'static str {
        match self {
            Self::All => "계정",
            Self::Admins => "관리자 계정",
            Self::Banned => "정지된 계정",
        }
    }

    fn keeps(self, account: &Account) -> bool {
        match self {
            Self::All => true,
            Self::Admins => account.admin,
            Self::Banned => account.banned,
        }
    }
}

pub fn accounts(accounts: &[Account], filter: AccountFilter, page: usize) -> Components {
    let mut matching: Vec<&Account> = accounts
        .iter()
        .filter(|account| filter.keeps(account))
        .collect();
    matching.sort_by(|left, right| left.nickname.cmp(&right.nickname));
    let pages = matching.len().div_ceil(ACCOUNTS_PER_PAGE).max(1);
    let page = page.min(pages - 1);
    let lines: Vec<String> = matching
        .iter()
        .skip(page * ACCOUNTS_PER_PAGE)
        .take(ACCOUNTS_PER_PAGE)
        .map(|account| {
            let flags = account_flags(account);
            if flags.is_empty() {
                escape(&account.nickname)
            } else {
                format!("{} · {flags}", escape(&account.nickname))
            }
        })
        .collect();
    let mut card = Card::new(BLURPLE)
        .text(format!("### {} {}", filter.title(), matching.len()))
        .text(fit_lines(&lines, 3_300, "없음"));
    if pages > 1 {
        let filter_key = filter.key();
        card = card.text(format!("-# {}/{pages}", page + 1)).buttons(vec![
            button(
                format!("acct:page:{filter_key}:{}", page.saturating_sub(1)),
                "이전",
                ButtonStyle::Secondary,
            )
            .disabled(page == 0),
            button(
                format!("acct:page:{filter_key}:{}", page + 1),
                "다음",
                ButtonStyle::Secondary,
            )
            .disabled(page + 1 >= pages),
        ]);
    }
    card.build()
}

pub fn account(info: &AccountInfo, notice: Option<&Notice>) -> Components {
    let account = &info.account;
    let nickname = &account.nickname;
    let accent = if account.banned {
        RED
    } else if info.online.is_some() {
        GREEN
    } else {
        GREY
    };
    let mut card = Card::new(accent);
    if let Some(notice) = notice {
        card = card.text(notice.line()).divider();
    }
    let mut lines = vec![format!("가입 {}", timestamp(account.created, 'D'))];
    if let Some(login) = info.last_login {
        lines.push(format!(
            "최근 로그인 {}{}",
            timestamp(login, 'R'),
            info.last_login_address
                .as_deref()
                .map_or_else(String::new, |address| format!(" · {}", code(address)))
        ));
    }
    lines.push(match &info.online {
        Some(player) => format!(
            "접속 중 · {} · {}",
            code(&player.address),
            player
                .room
                .map_or_else(|| "로비".to_owned(), |room| format!("#{room}"))
        ),
        None => "오프라인".to_owned(),
    });
    let mut first = vec![button(
        format!("acct:refresh:{nickname}"),
        "새로고침",
        ButtonStyle::Secondary,
    )];
    if info.online.is_some() {
        first.push(button(
            format!("acct:kick:{nickname}"),
            "퇴장",
            ButtonStyle::Secondary,
        ));
    }
    first.push(if account.banned {
        button(
            format!("acct:unban:{nickname}"),
            "정지 해제",
            ButtonStyle::Success,
        )
    } else {
        button(format!("acct:ban:{nickname}"), "정지", ButtonStyle::Danger)
    });
    let second = vec![
        if account.admin {
            button(
                format!("acct:admin:0:{nickname}"),
                "관리자 해제",
                ButtonStyle::Secondary,
            )
        } else {
            button(
                format!("acct:admin:1:{nickname}"),
                "관리자 지정",
                ButtonStyle::Secondary,
            )
        },
        button(
            format!("acct:pw:{nickname}"),
            "비밀번호 변경",
            ButtonStyle::Secondary,
        ),
        button(
            format!("acct:delete:{nickname}"),
            "삭제",
            ButtonStyle::Danger,
        ),
    ];
    card.text(format!("## {}", escape(nickname)))
        .text(account_flags(account))
        .text(lines.join("\n"))
        .buttons(first)
        .buttons(second)
        .build()
}

pub fn address_bans(bans: &[AddressBan]) -> Components {
    let lines: Vec<String> = bans
        .iter()
        .map(|ban| match &ban.reason {
            Some(reason) => format!("{} · {}", code(&ban.address), escape(reason)),
            None => code(&ban.address),
        })
        .collect();
    Card::new(BLURPLE)
        .text(format!("### 차단한 IP {}", bans.len()))
        .text(fit_lines(&lines, 3_500, "없음"))
        .build()
}

pub fn rewards(rewards: &Rewards) -> Components {
    let race = |room: u32, epoch: u64, nickname: Option<&str>| {
        format!(
            "방 #{room} · 경기 {epoch} · {}",
            nickname.map_or_else(|| "-".to_owned(), escape)
        )
    };
    let outstanding: Vec<String> = rewards
        .outstanding
        .iter()
        .map(|lane| race(lane.room, lane.race_epoch, lane.nickname.as_deref()))
        .collect();
    let dead_letters: Vec<String> = rewards
        .dead_letters
        .iter()
        .map(|dead| {
            let line = race(dead.room, dead.race_epoch, dead.nickname.as_deref());
            if dead.retryable {
                format!("{line} · 재시도 가능")
            } else {
                line
            }
        })
        .collect();
    let mut card = Card::new(if rewards.dead_letters.is_empty() {
        GREEN
    } else {
        ORANGE
    })
    .text(format!("### 저장 중 {}", outstanding.len()))
    .text(fit_lines(&outstanding, 1_500, "없음"))
    .divider()
    .text(format!("### 저장 실패 {}", dead_letters.len()))
    .text(fit_lines(&dead_letters, 1_800, "없음"));
    if rewards.dead_letters.iter().any(|dead| dead.retryable) {
        card = card.buttons(vec![button(
            "rewards:retry",
            "모두 재시도",
            ButtonStyle::Primary,
        )]);
    }
    card.build()
}

pub fn throttles(throttles: &Throttles) -> Components {
    let limited = |limited: bool| if limited { " · 제한됨" } else { "" };
    let launcher: Vec<String> = throttles
        .launcher
        .iter()
        .map(|entry| {
            format!(
                "{} · 로그인 실패 {} · 가입 {}{}",
                code(&entry.address),
                entry.failed_logins,
                entry.registrations,
                limited(entry.limited)
            )
        })
        .collect();
    let console: Vec<String> = throttles
        .console
        .iter()
        .map(|entry| {
            format!(
                "{} · 토큰 실패 {}{}",
                code(&entry.address),
                entry.failed_tokens,
                limited(entry.limited)
            )
        })
        .collect();
    let mut card = Card::new(BLURPLE)
        .text(format!("### 실행기 {}", launcher.len()))
        .text(fit_lines(&launcher, 1_700, "없음"))
        .divider()
        .text(format!("### 콘솔 {}", console.len()))
        .text(fit_lines(&console, 1_700, "없음"));
    if !launcher.is_empty() || !console.is_empty() {
        card = card.buttons(vec![button(
            "throttles:clear",
            "모두 초기화",
            ButtonStyle::Secondary,
        )]);
    }
    card.build()
}

pub fn audit(entries: &[AuditEntry]) -> Components {
    let lines: Vec<String> = entries
        .iter()
        .rev()
        .map(|entry| {
            let mut line = format!(
                "{} **{}** {}",
                timestamp(entry.time, 'f'),
                escape(&entry.action),
                code(&entry.peer),
            );
            if !entry.detail.is_null() {
                let mut detail = entry.detail.to_string();
                if detail.chars().count() > 120 {
                    detail = detail.chars().take(119).collect::<String>() + "…";
                }
                let _ = write!(line, "\n-# {}", code(&detail));
            }
            line
        })
        .collect();
    Card::new(BLURPLE)
        .text(format!("### 운영 기록 {}", entries.len()))
        .text(fit_lines(&lines, 3_500, "없음"))
        .build()
}

pub fn panel(
    status: &Status,
    players: &[Player],
    rewards: &Rewards,
    throttles: &Throttles,
    notice: Option<&Notice>,
) -> Components {
    let mut card = Card::new(status_colour(status));
    if let Some(notice) = notice {
        card = card.text(notice.line()).divider();
    }
    let retryable = rewards
        .dead_letters
        .iter()
        .filter(|dead| dead.retryable)
        .count();
    let attempts = throttles.launcher.len() + throttles.console.len();
    card = card
        .text("## 운영 패널")
        .section(
            format!("**점검 모드** {}", on_off(status.maintenance)),
            if status.maintenance {
                button("panel:maint:off", "끄기", ButtonStyle::Success)
            } else {
                button("panel:maint:on", "켜기", ButtonStyle::Secondary)
            },
        )
        .section(
            format!("**가입** {}", open_closed(status.registration_open)),
            if status.registration_open {
                button("panel:reg:close", "닫기", ButtonStyle::Secondary)
            } else {
                button("panel:reg:open", "열기", ButtonStyle::Secondary)
            },
        )
        .section(
            format!("**보상 저장 실패** {}", rewards.dead_letters.len()),
            button("panel:retry", "재시도", ButtonStyle::Secondary).disabled(retryable == 0),
        )
        .section(
            format!("**시도 제한** {attempts}"),
            button("panel:clearthr", "초기화", ButtonStyle::Secondary).disabled(attempts == 0),
        )
        .divider();
    if !players.is_empty() {
        let options: Vec<CreateSelectMenuOption<'static>> = sorted_players(players)
            .into_iter()
            .take(25)
            .map(|player| {
                CreateSelectMenuOption::new(player.nickname.clone(), player.nickname.clone())
                    .description(player.address.clone())
            })
            .collect();
        card = card.select(
            CreateSelectMenu::new(
                "panel:kick",
                CreateSelectMenuKind::String {
                    options: options.into(),
                },
            )
            .placeholder("퇴장시킬 라이더"),
        );
    }
    card.buttons(vec![
        button("panel:refresh", "새로고침", ButtonStyle::Secondary),
        button("panel:kickall:keep", "관리자 외 퇴장", ButtonStyle::Danger),
        button("panel:kickall:all", "전원 퇴장", ButtonStyle::Danger),
        button("panel:shutdown", "서버 종료", ButtonStyle::Danger),
    ])
    .build()
}

pub fn help() -> Components {
    Card::new(BLURPLE)
        .text("## 명령어")
        .text(
            "`/status` `/online` `/rooms` `/players` `/ping`\n\
             `/dashboard` `/panel` `/settings`\n\
             `/account` `/ban` `/unban` `/kick`\n\
             `/maintenance` `/registration` `/rewards` `/throttles`\n\
             `/audit` `/shutdown` `/purge`",
        )
        .build()
}

fn channel_select(
    id: &'static str,
    current: Option<GenericChannelId>,
) -> CreateSelectMenu<'static> {
    CreateSelectMenu::new(
        id,
        CreateSelectMenuKind::Channel {
            channel_types: Some(
                vec![
                    ChannelType::Text,
                    ChannelType::News,
                    ChannelType::PublicThread,
                ]
                .into(),
            ),
            default_channels: current.map(|channel| vec![channel].into()),
        },
    )
    .placeholder("채널 선택")
}

pub fn settings(settings: &Settings, notice: Option<&Notice>) -> Components {
    let channel = |channel: Option<GenericChannelId>| {
        channel.map_or_else(|| "꺼짐".to_owned(), |channel| format!("<#{channel}>"))
    };
    let mut card = Card::new(BLURPLE);
    if let Some(notice) = notice {
        card = card.text(notice.line()).divider();
    }
    let poll_options: Vec<CreateSelectMenuOption<'static>> = POLL_CHOICES
        .iter()
        .map(|seconds| {
            CreateSelectMenuOption::new(format!("{seconds}초"), seconds.to_string())
                .default_selection(*seconds == settings.poll_seconds)
        })
        .collect();
    card.text("## 설정")
        .section(
            format!("**알림 채널** {}", channel(settings.alert_channel)),
            button("set:alert:off", "끄기", ButtonStyle::Secondary)
                .disabled(settings.alert_channel.is_none()),
        )
        .select(channel_select("set:alert", settings.alert_channel))
        .section(
            format!("**접속·퇴장 알림** {}", on_off(settings.join_alerts)),
            if settings.join_alerts {
                button("set:join:0", "끄기", ButtonStyle::Secondary)
            } else {
                button("set:join:1", "켜기", ButtonStyle::Secondary)
            },
        )
        .divider()
        .section(
            format!("**운영 기록 채널** {}", channel(settings.audit_channel)),
            button("set:audit:off", "끄기", ButtonStyle::Secondary)
                .disabled(settings.audit_channel.is_none()),
        )
        .select(channel_select("set:audit", settings.audit_channel))
        .divider()
        .text(format!("**갱신 간격** {}초", settings.poll_seconds))
        .select(CreateSelectMenu::new(
            "set:poll",
            CreateSelectMenuKind::String {
                options: poll_options.into(),
            },
        ))
        .build()
}
