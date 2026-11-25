use std::fmt::Write as _;

use crate::console::KickedReply;

pub fn timestamp(unix: u64, style: char) -> String {
    format!("<t:{unix}:{style}>")
}

pub fn elapsed(seconds: u64) -> String {
    match seconds {
        0..60 => format!("{seconds}초"),
        60..3_600 => format!("{}분", seconds / 60),
        3_600..86_400 => format!("{}시간 {}분", seconds / 3_600, seconds % 3_600 / 60),
        _ => format!("{}일 {}시간", seconds / 86_400, seconds % 86_400 / 3_600),
    }
}

pub fn mode_label(game_type: u8, speed_type: u8) -> &'static str {
    match (game_type, speed_type) {
        (1, 4) => "무한부스터 개인전",
        (3, 4) => "무한부스터 팀전",
        (1, _) => "스피드 개인전",
        (3, _) => "스피드 팀전",
        (2, _) => "아이템 개인전",
        (4, _) => "아이템 팀전",
        _ => "카트라이더",
    }
}

pub fn phase_label(phase: &str) -> &'static str {
    match phase {
        "lobby" => "대기",
        "loading" => "로딩",
        "running" => "경기 중",
        "settling" => "결과",
        _ => "-",
    }
}

pub fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if matches!(
            character,
            '\\' | '*'
                | '_'
                | '~'
                | '`'
                | '|'
                | '>'
                | '<'
                | '#'
                | '['
                | ']'
                | '('
                | ')'
                | '-'
                | '@'
        ) {
            escaped.push('\\');
        }
        if character.is_control() {
            escaped.push(' ');
        } else {
            escaped.push(character);
        }
    }
    escaped
}

pub fn code(text: &str) -> String {
    format!("`{}`", text.replace('`', "ˋ"))
}

const DURATION_HINT: &str = "기간은 30m, 12h, 7d 형식";

pub fn parse_duration(text: &str) -> Result<u64, String> {
    let text = text.trim();
    let split = text
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    let number: u64 = number.parse().map_err(|_| DURATION_HINT.to_owned())?;
    let scale = match unit.trim() {
        "" | "s" | "초" => 1,
        "m" | "분" => 60,
        "h" | "시간" => 60 * 60,
        "d" | "일" => 24 * 60 * 60,
        "w" | "주" => 7 * 24 * 60 * 60,
        _ => return Err(DURATION_HINT.to_owned()),
    };
    number
        .checked_mul(scale)
        .filter(|seconds| *seconds > 0)
        .ok_or_else(|| DURATION_HINT.to_owned())
}

pub fn fit_lines(lines: &[String], budget: usize, empty: &str) -> String {
    if lines.is_empty() {
        return empty.to_owned();
    }
    let mut text = String::new();
    let mut used = 0;
    for (index, line) in lines.iter().enumerate() {
        let length = line.chars().count() + 1;
        if used + length > budget.saturating_sub(16) && index + 1 < lines.len() {
            let _ = write!(text, "-# … 외 {}개", lines.len() - index);
            return text;
        }
        text.push_str(line);
        text.push('\n');
        used += length;
    }
    text.pop();
    text
}

pub fn kicked(reply: &KickedReply) -> Option<String> {
    let (sessions, nicknames) = reply.report();
    if sessions == 0 {
        None
    } else if nicknames.is_empty() {
        Some(format!("연결 {sessions}개 종료"))
    } else {
        let names = nicknames
            .iter()
            .map(|name| escape(name))
            .collect::<Vec<_>>()
            .join(", ");
        Some(format!("퇴장: {names}"))
    }
}

pub fn on_off(on: bool) -> &'static str {
    if on { "켜짐" } else { "꺼짐" }
}

pub fn open_closed(open: bool) -> &'static str {
    if open { "열림" } else { "닫힘" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_durations() {
        assert_eq!(parse_duration("90"), Ok(90));
        assert_eq!(parse_duration("30m"), Ok(1_800));
        assert_eq!(parse_duration("12시간"), Ok(43_200));
        assert_eq!(parse_duration("7d"), Ok(604_800));
        assert_eq!(parse_duration("2주"), Ok(1_209_600));
        assert!(parse_duration("0").is_err());
        assert!(parse_duration("5y").is_err());
        assert!(parse_duration("h").is_err());
    }

    #[test]
    fn formats_text() {
        assert_eq!(elapsed(3_725), "1시간 2분");
        assert_eq!(elapsed(59), "59초");
        assert_eq!(escape("a_b*c"), "a\\_b\\*c");
        assert_eq!(mode_label(1, 4), "무한부스터 개인전");
        assert_eq!(mode_label(2, 0), "아이템 개인전");
    }

    #[test]
    fn fits_lines_into_a_budget() {
        let lines: Vec<String> = (0..100).map(|index| format!("line {index}")).collect();
        let text = fit_lines(&lines, 100, "-");
        assert!(text.chars().count() <= 100);
        assert!(text.ends_with("개"));
        assert_eq!(fit_lines(&[], 100, "없음"), "없음");
        assert_eq!(
            fit_lines(&["a".to_owned(), "b".to_owned()], 100, "-"),
            "a\nb"
        );
    }
}
