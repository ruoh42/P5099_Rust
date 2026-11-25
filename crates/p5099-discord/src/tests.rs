use std::sync::Arc;

use serde_json::{Value, json};
use serenity::model::id::GenericChannelId;

use crate::{
    actions,
    console::{
        Account, AccountInfo, AddressBan, AuditEntry, Backend, Console, ConsoleError, Player,
        Reply, Rewards, Room, Status, Throttles, rcon,
    },
    monitor::{Health, Settings, Snapshot},
    views::{self, AccountFilter, Components, Notice, Tone},
};

const MAX_COMPONENTS: usize = 40;
const MAX_TEXT: usize = 4_000;

fn check(components: &Components) {
    fn walk(value: &Value, count: &mut usize, text: &mut usize) {
        match value {
            Value::Object(map) => {
                if let Some(kind) = map.get("type").and_then(Value::as_u64) {
                    *count += 1;
                    match kind {
                        1 => {
                            let children = map["components"].as_array().unwrap();
                            assert!((1..=5).contains(&children.len()), "action row size");
                        }
                        2 | 3 => {
                            if let Some(id) = map.get("custom_id").and_then(Value::as_str) {
                                assert!(id.chars().count() <= 100, "custom_id too long: {id}");
                            }
                            if let Some(options) = map.get("options").and_then(Value::as_array) {
                                assert!((1..=25).contains(&options.len()), "select options");
                            }
                        }
                        9 => {
                            let children = map["components"].as_array().unwrap();
                            assert!((1..=3).contains(&children.len()), "section texts");
                            assert!(map.contains_key("accessory"));
                        }
                        10 => {
                            let content = map["content"].as_str().unwrap();
                            assert!(!content.trim().is_empty(), "empty text display");
                            *text += content.chars().count();
                        }
                        _ => {}
                    }
                }
                for child in map.values() {
                    walk(child, count, text);
                }
            }
            Value::Array(items) => {
                for item in items {
                    walk(item, count, text);
                }
            }
            _ => {}
        }
    }
    let value = serde_json::to_value(components).unwrap();
    let (mut count, mut text) = (0, 0);
    walk(&value, &mut count, &mut text);
    assert!(count <= MAX_COMPONENTS, "{count} components");
    assert!(text <= MAX_TEXT, "{text} characters of text");
}

fn status(players: u64, rooms: u64) -> Status {
    serde_json::from_value(json!({
        "version": "1.1.1", "server_time": 1_791_504_000, "uptime_seconds": 3_725,
        "sessions": players + 2, "players_online": players, "observers_online": 3,
        "rooms": rooms, "racing_rooms": rooms / 2, "accounts": 900,
        "banned_accounts": 12, "require_account": true,
        "registration_open": true, "maintenance": true,
        "reward_recovery": { "outstanding_lanes": 2, "dead_letters": 5 },
    }))
    .unwrap()
}

fn name(index: usize) -> String {
    format!("**_라이더~{index:04}_**")
}

fn crowd(players: usize, rooms: usize) -> Snapshot {
    let rooms: Vec<Room> = (0..rooms)
        .map(|index| Room {
            id: u32::try_from(index).unwrap() + 1,
            name: format!("아주 긴 방 이름입니다 `{index}` | 들어오세요"),
            game_type: u8::try_from(index % 4).unwrap() + 1,
            speed_type: 4,
            track: 100_000 + u32::try_from(index).unwrap(),
            phase: ["lobby", "loading", "running", "settling"][index % 4].to_owned(),
            locked: index % 3 == 0,
            players: (0..8).map(|seat| name(index * 8 + seat)).collect(),
            observers: (0..3).map(|seat| name(10_000 + seat)).collect(),
            capacity: 8,
        })
        .collect();
    let players: Vec<Player> = (0..players)
        .map(|index| Player {
            nickname: name(index),
            user_no: u32::try_from(index).unwrap(),
            address: format!("2001:db8::{index:x}"),
            room: (index % 2 == 0).then(|| u32::try_from(index % rooms.len().max(1)).unwrap() + 1),
            admin: index % 7 == 0,
        })
        .collect();
    let status = status(players.len() as u64, rooms.len() as u64);
    Snapshot {
        started: status.started(),
        status,
        players,
        rooms,
    }
}

#[test]
#[allow(clippy::too_many_lines, reason = "one check per layout")]
fn layouts_fit_discord_limits() {
    for (players, rooms) in [(0, 0), (1, 1), (60, 12), (400, 120)] {
        let snapshot = crowd(players, rooms);
        let health = Health::Online(Arc::new(snapshot.clone()));
        check(&views::dashboard(&health, 1));
        check(&views::status(snapshot.clone()));
        check(&views::online(&snapshot.players, &snapshot.rooms));
        check(&views::rooms(&snapshot.rooms));
        check(&views::players(&snapshot.players, &snapshot.rooms));
        let rewards: Rewards = serde_json::from_value(json!({
            "outstanding": (0..players).map(|index| json!({
                "room": index, "race_epoch": index, "nickname": name(index), "phase": "persisting",
            })).collect::<Vec<_>>(),
            "dead_letters": (0..players).map(|index| json!({
                "room": index, "race_epoch": index, "nickname": name(index),
                "reason": "reward_persistence", "retryable": true,
            })).collect::<Vec<_>>(),
        }))
        .unwrap();
        let throttles: Throttles = serde_json::from_value(json!({
            "launcher": (0..players).map(|index| json!({
                "address": format!("10.0.{}.{}", index / 250, index % 250),
                "failed_logins": 9, "registrations": 2, "limited": true,
            })).collect::<Vec<_>>(),
            "console": (0..players).map(|index| json!({
                "address": format!("10.1.{}.{}", index / 250, index % 250),
                "failed_tokens": 9, "limited": true,
            })).collect::<Vec<_>>(),
        }))
        .unwrap();
        let notice = Notice::new(Tone::Success, "x".repeat(300));
        check(&views::panel(
            &snapshot.status,
            &snapshot.players,
            &rewards,
            &throttles,
            Some(&notice),
        ));
        check(&views::rewards(&rewards));
        check(&views::throttles(&throttles));
        let accounts: Vec<Account> = (0..players)
            .map(|index| Account {
                nickname: name(index),
                created: 1,
                admin: index % 5 == 0,
                banned: index % 3 == 0,
                ban_reason: Some("사유".repeat(60)),
                banned_until: Some(2),
            })
            .collect();
        for filter in [
            AccountFilter::All,
            AccountFilter::Admins,
            AccountFilter::Banned,
        ] {
            for page in [0, 1, 999] {
                check(&views::accounts(&accounts, filter, page));
            }
        }
        let bans: Vec<AddressBan> = (0..players)
            .map(|index| AddressBan {
                address: format!("2001:db8::{index:x}"),
                reason: Some("이유".repeat(50)),
            })
            .collect();
        check(&views::address_bans(&bans));
        let audit: Vec<AuditEntry> = (0..players.min(256))
            .map(|index| AuditEntry {
                time: 1,
                peer: "127.0.0.1".to_owned(),
                action: "ban".to_owned(),
                detail: json!({ "nickname": name(index), "reason": "x".repeat(300) }),
            })
            .collect();
        check(&views::audit(&audit));
    }
    let info = AccountInfo {
        account: Account {
            nickname: name(1),
            created: 1,
            admin: true,
            banned: true,
            ban_reason: Some("사유".repeat(100)),
            banned_until: Some(2),
        },
        last_login: Some(1),
        last_login_address: Some("2001:db8::1".to_owned()),
        online: crowd(1, 1).players.pop(),
    };
    check(&views::account(
        &info,
        Some(&Notice::new(Tone::Error, "y".repeat(500))),
    ));
    check(&views::dashboard(&Health::Starting, 1));
    check(&views::dashboard(
        &Health::Offline {
            since: 1,
            error: "연결 거부".to_owned(),
        },
        1,
    ));
    check(&views::help());
    check(&views::settings(&Settings::default(), None));
    check(&views::settings(
        &Settings {
            alert_channel: Some(GenericChannelId::new(1)),
            audit_channel: Some(GenericChannelId::new(2)),
            join_alerts: true,
            poll_seconds: 60,
        },
        Some(&Notice::new(Tone::Error, "z".repeat(500))),
    ));
    check(&views::confirm("제목", "delete:라이더"));
}

struct Rcon {
    address: String,
    token: String,
}

impl Backend for Rcon {
    fn execute(&self, _caller: String, line: String) -> Reply {
        let (address, token) = (self.address.clone(), self.token.clone());
        Box::pin(async move {
            let reply = rcon::exchange(&address, &token, &line).await?;
            serde_json::from_slice(&reply).map_err(|error| ConsoleError::Decode(error.to_string()))
        })
    }
}

fn live_console(token: String) -> Console {
    Console::new(Arc::new(Rcon {
        address: std::env::var("P5099_ADMIN_ADDRESS")
            .unwrap_or_else(|_| "127.0.0.1:39318".to_owned()),
        token,
    }))
}

#[tokio::test]
#[ignore = "needs a running p5099-server"]
async fn every_console_call_works_against_a_live_server() {
    let console = live_console(std::env::var("P5099_ADMIN_TOKEN").expect("set P5099_ADMIN_TOKEN"));
    let ok = |action: actions::Action| {
        let summary = action
            .outcome
            .unwrap_or_else(|error| panic!("{}: {error}", action.label));
        println!("ok  {} → {summary}", action.label);
    };
    let refused = |action: actions::Action| {
        let error = action.outcome.expect_err(&action.label);
        println!("err {} → {error}", action.label);
        error
    };

    let snapshot = Snapshot::fetch(&console).await.unwrap();
    check(&views::status(snapshot.clone()));
    check(&views::dashboard(
        &Health::Online(Arc::new(snapshot.clone())),
        1,
    ));
    check(&views::players(&snapshot.players, &snapshot.rooms));

    let nickname = "디코봇테스트";
    let _ = console.delete_account(nickname).await;
    ok(actions::create_account(&console, nickname, "password1234", false).await);
    assert!(
        refused(actions::create_account(&console, nickname, "password1234", false).await)
            .contains("이미 있는 닉네임")
    );
    assert!(
        refused(actions::create_account(&console, "짧은비번", "abc", false).await)
            .contains("8~64자")
    );
    ok(actions::set_admin(&console, nickname, true).await);
    ok(actions::ban(&console, nickname, Some("테스트"), Some("30m")).await);
    let info = console.account_info(nickname).await.unwrap();
    assert!(info.account.admin && info.account.banned && info.account.banned_until.is_some());
    check(&views::account(&info, None));
    refused(actions::ban(&console, nickname, None, Some("5y")).await);
    ok(actions::unban(&console, nickname).await);
    ok(actions::reset_password(&console, nickname, "another-password").await);
    let accounts = console.accounts().await.unwrap();
    assert!(accounts.iter().any(|account| account.nickname == nickname));
    check(&views::accounts(&accounts, AccountFilter::Admins, 0));

    ok(actions::ban_address(&console, "203.0.113.9", Some("테스트")).await);
    let bans = console.address_bans().await.unwrap();
    assert!(bans.iter().any(|ban| ban.address == "203.0.113.9"));
    check(&views::address_bans(&bans));
    ok(actions::unban_address(&console, "203.0.113.9").await);
    refused(actions::ban_address(&console, "not-an-ip", None).await);

    assert!(refused(actions::kick(&console, nickname).await).contains("접속 중이 아님"));
    assert!(refused(actions::kick_room(&console, 99_999).await).contains("방 없음"));
    ok(actions::kick_address(&console, "203.0.113.10").await);
    ok(actions::kick_all(&console, true).await);

    ok(actions::set_maintenance(&console, true, true).await);
    assert!(console.status().await.unwrap().maintenance);
    ok(actions::set_maintenance(&console, false, false).await);
    assert!(!console.status().await.unwrap().maintenance);
    ok(actions::set_registration(&console, false).await);
    assert!(!console.status().await.unwrap().registration_open);
    ok(actions::set_registration(&console, true).await);

    check(&views::rewards(&console.rewards().await.unwrap()));
    ok(actions::retry_rewards(&console, None).await);
    refused(actions::retry_rewards(&console, Some((1, 1))).await);
    check(&views::throttles(&console.throttles().await.unwrap()));
    ok(actions::clear_throttles(&console, None).await);
    ok(actions::clear_throttles(&console, Some("127.0.0.1")).await);
    let audit = console.audit(50).await.unwrap();
    assert!(audit.iter().any(|entry| entry.action.contains("ban")));
    check(&views::audit(&audit));

    ok(actions::delete_account(&console, nickname).await);
    assert!(refused(actions::delete_account(&console, nickname).await).contains("계정 없음"));

    let wrong = live_console("wrong-token-wrong-token".to_owned());
    let error = wrong.status().await.unwrap_err().to_string();
    assert!(error.contains("토큰"), "{error}");
}
