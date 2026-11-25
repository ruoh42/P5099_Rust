use std::net::IpAddr;

use crate::{
    console::{Console, ConsoleError, KickedReply},
    format::{code, escape, kicked, parse_duration, timestamp},
};

pub struct Action {
    pub label: String,
    pub outcome: Result<String, String>,
}

impl Action {
    fn new(label: impl Into<String>, outcome: Result<String, ConsoleError>) -> Self {
        Self {
            label: label.into(),
            outcome: outcome.map_err(|error| error.to_string()),
        }
    }

    pub fn refused(label: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            outcome: Err(reason.into()),
        }
    }
}

fn name(nickname: &str) -> String {
    format!("**{}**", escape(nickname))
}

fn with_kicked(text: String, reply: &KickedReply) -> String {
    match kicked(reply) {
        Some(kicked) => format!("{text} · {kicked}"),
        None => text,
    }
}

fn address(text: &str) -> Result<String, String> {
    text.trim()
        .parse::<IpAddr>()
        .map(|address| address.to_string())
        .map_err(|_| format!("{}은(는) IP 주소가 아닙니다", code(text)))
}

pub async fn kick(console: &Console, nickname: &str) -> Action {
    Action::new(
        format!("퇴장 {}", escape(nickname)),
        console
            .kick(nickname)
            .await
            .map(|_| format!("{} 퇴장", name(nickname))),
    )
}

pub async fn kick_address(console: &Console, text: &str) -> Action {
    let label = format!("주소 퇴장 {}", code(text));
    match address(text) {
        Ok(address) => Action::new(
            label,
            console
                .kick_address(&address)
                .await
                .map(|reply| kicked(&reply).unwrap_or_else(|| "끊을 연결 없음".to_owned())),
        ),
        Err(error) => Action::refused(label, error),
    }
}

pub async fn kick_room(console: &Console, id: u32) -> Action {
    Action::new(
        format!("방 퇴장 #{id}"),
        console
            .kick_room(id)
            .await
            .map(|reply| kicked(&reply).unwrap_or_else(|| "빈 방".to_owned())),
    )
}

pub async fn kick_all(console: &Console, keep_admins: bool) -> Action {
    Action::new(
        if keep_admins {
            "관리자 외 퇴장"
        } else {
            "전원 퇴장"
        },
        console
            .kick_all(keep_admins)
            .await
            .map(|reply| kicked(&reply).unwrap_or_else(|| "접속자 없음".to_owned())),
    )
}

pub async fn ban(
    console: &Console,
    nickname: &str,
    reason: Option<&str>,
    duration: Option<&str>,
) -> Action {
    let label = format!(
        "정지 {}{}{}",
        escape(nickname),
        duration.map_or_else(String::new, |duration| format!(" · {}", code(duration))),
        reason.map_or_else(String::new, |reason| format!(" · {}", escape(reason))),
    );
    let seconds = match duration.map(parse_duration).transpose() {
        Ok(seconds) => seconds,
        Err(error) => return Action::refused(label, error),
    };
    Action::new(
        label,
        console.ban(nickname, reason, seconds).await.map(|reply| {
            let text = match reply.banned_until {
                Some(until) => format!("{} 정지 ~{}", name(&reply.nickname), timestamp(until, 'f')),
                None => format!("{} 정지", name(&reply.nickname)),
            };
            with_kicked(text, &reply.kicked)
        }),
    )
}

pub async fn unban(console: &Console, nickname: &str) -> Action {
    Action::new(
        format!("정지 해제 {}", escape(nickname)),
        console
            .unban(nickname)
            .await
            .map(|reply| format!("{} 정지 해제", name(&reply.nickname))),
    )
}

pub async fn ban_address(console: &Console, text: &str, reason: Option<&str>) -> Action {
    let label = format!(
        "IP 차단 {}{}",
        code(text),
        reason.map_or_else(String::new, |reason| format!(" · {}", escape(reason)))
    );
    match address(text) {
        Ok(address) => Action::new(
            label,
            console
                .ban_address(&address, reason)
                .await
                .map(|reply| with_kicked(format!("{} 차단", code(&reply.address)), &reply.kicked)),
        ),
        Err(error) => Action::refused(label, error),
    }
}

pub async fn unban_address(console: &Console, text: &str) -> Action {
    let label = format!("IP 차단 해제 {}", code(text));
    match address(text) {
        Ok(address) => Action::new(
            label,
            console.unban_address(&address).await.map(|reply| {
                if reply.changed {
                    format!("{} 차단 해제", code(&reply.address))
                } else {
                    format!("{}은(는) 차단되어 있지 않습니다", code(&reply.address))
                }
            }),
        ),
        Err(error) => Action::refused(label, error),
    }
}

pub async fn set_admin(console: &Console, nickname: &str, admin: bool) -> Action {
    let verb = if admin {
        "관리자 지정"
    } else {
        "관리자 해제"
    };
    Action::new(
        format!("{verb} {}", escape(nickname)),
        console
            .set_admin(nickname, admin)
            .await
            .map(|reply| format!("{} {verb}", name(&reply.nickname))),
    )
}

pub async fn create_account(
    console: &Console,
    nickname: &str,
    password: &str,
    admin: bool,
) -> Action {
    Action::new(
        format!(
            "계정 생성 {}{}",
            escape(nickname),
            if admin { " (관리자)" } else { "" }
        ),
        console
            .create_account(nickname, password, admin)
            .await
            .map(|reply| format!("{} 계정 생성", name(&reply.nickname))),
    )
}

pub async fn reset_password(console: &Console, nickname: &str, password: &str) -> Action {
    Action::new(
        format!("비밀번호 변경 {}", escape(nickname)),
        console
            .reset_password(nickname, password)
            .await
            .map(|reply| format!("{} 비밀번호 변경", name(&reply.nickname))),
    )
}

pub async fn delete_account(console: &Console, nickname: &str) -> Action {
    Action::new(
        format!("계정 삭제 {}", escape(nickname)),
        console.delete_account(nickname).await.map(|reply| {
            with_kicked(
                format!(
                    "{} 계정 삭제",
                    name(reply.nickname.as_deref().unwrap_or(nickname))
                ),
                &reply,
            )
        }),
    )
}

pub async fn set_maintenance(console: &Console, enabled: bool, kick: bool) -> Action {
    let label = match (enabled, kick) {
        (true, true) => "점검 켜기 · 관리자 외 퇴장",
        (true, false) => "점검 켜기",
        (false, _) => "점검 끄기",
    };
    Action::new(
        label,
        console.set_maintenance(enabled, kick).await.map(|reply| {
            let text = if enabled {
                "점검 모드 켜짐"
            } else {
                "점검 모드 꺼짐"
            };
            with_kicked(text.to_owned(), &reply)
        }),
    )
}

pub async fn set_registration(console: &Console, open: bool) -> Action {
    let text = if open {
        "가입 열림"
    } else {
        "가입 닫힘"
    };
    Action::new(
        if open {
            "가입 열기"
        } else {
            "가입 닫기"
        },
        console
            .set_registration(open)
            .await
            .map(|()| text.to_owned()),
    )
}

pub async fn retry_rewards(console: &Console, race: Option<(u32, u64)>) -> Action {
    Action::new(
        race.map_or_else(
            || "보상 재시도".to_owned(),
            |(room, epoch)| format!("보상 재시도 #{room} 경기 {epoch}"),
        ),
        console.retry_rewards(race).await.map(|reply| {
            if reply.retried.is_empty() && reply.failed.is_empty() {
                return "재시도할 항목 없음".to_owned();
            }
            let mut lines: Vec<String> = reply
                .retried
                .iter()
                .map(|race| format!("성공 · 방 #{} 경기 {}", race.room, race.race_epoch))
                .collect();
            lines.extend(reply.failed.iter().map(|race| {
                format!(
                    "실패 · 방 #{} 경기 {} · {}",
                    race.room,
                    race.race_epoch,
                    escape(race.error.as_deref().unwrap_or("-"))
                )
            }));
            lines.join("\n")
        }),
    )
}

pub async fn clear_throttles(console: &Console, text: Option<&str>) -> Action {
    let label = text.map_or_else(
        || "시도 제한 초기화".to_owned(),
        |text| format!("시도 제한 초기화 {}", code(text)),
    );
    let target = match text.map(address).transpose() {
        Ok(target) => target,
        Err(error) => return Action::refused(label, error),
    };
    Action::new(
        label,
        console
            .clear_throttles(target.as_deref())
            .await
            .map(|()| "시도 기록 초기화".to_owned()),
    )
}

pub async fn shutdown(console: &Console) -> Action {
    Action::new(
        "서버 종료",
        console
            .shutdown()
            .await
            .map(|()| "서버 종료 요청".to_owned()),
    )
}
