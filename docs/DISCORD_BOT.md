# Discord 운영 봇 (`p5099-discord`)

[운영 콘솔](ADMIN_CONSOLE.md)의 모든 기능을 Discord에서 쓰는 봇입니다. `p5099-server` 프로세스 안에서 함께 돌고 콘솔 명령을 직접 실행하므로 RCON 토큰이 필요 없습니다.

- 봇을 업데이트하려면 게임 서버를 다시 시작해야 합니다(접속자가 모두 끊깁니다).
- 서버가 비정상 종료되면 봇도 멈추므로 그 알림은 오지 않습니다.
- 봇이 멈춰도 게임 서버는 계속 돕니다(로그: `Discord plugin stopped`).

## 기능

| 기능 | 설명 |
|---|---|
| 대시보드 | `/dashboard create`로 올리면 상태·접속자·방을 주기적으로 갱신합니다. `서버 알림` 스레드를 만들어 알림 채널로 정합니다. |
| 운영 패널 | `/panel`. 점검·가입·보상 재시도·시도 제한 초기화, 퇴장, 서버 종료 버튼. 본인에게만 보입니다. |
| 계정 카드 | `/account info`. 퇴장·정지·해제·관리자 지정·비밀번호 재설정·삭제 버튼. |
| 알림 | 서버 응답 없음/복구, 재시작, 점검·가입 변경, 보상 저장 실패, (선택) 접속·퇴장 |
| 운영 기록 | 봇으로 한 모든 변경과 실행자를 기록 채널에 남깁니다. |

비밀번호는 입력 창으로 받아 기록에 남지 않습니다. `/status`, `/online`, `/rooms`, `/ping`을 뺀 응답은 명령을 쓴 사람에게만 보입니다.

## 명령

| 명령 | `p5099-admin` |
|---|---|
| `/status`, `/rooms`, `/ping`, `/players` | `status`, `rooms`, `health`, `players` |
| `/online` | IP 없는 접속자 목록 |
| `/help`, `/dashboard create\|remove`, `/panel`, `/settings` | 봇 전용 |
| `/account list\|info\|create\|delete\|password\|admin` | `accounts`, `info`, `create`, `delete`, `password`, `admin`/`unadmin` |
| `/ban account\|ip\|list`, `/unban account\|ip` | `ban`, `ban-ip`, `bans`, `unban`, `unban-ip` |
| `/kick player\|ip\|room\|all` | `kick`, `kick-ip`, `kick-room`, `kick-all` |
| `/maintenance status\|on [kick]\|off`, `/registration` | `maintenance`, `registration` |
| `/rewards list\|retry`, `/throttles list\|clear`, `/audit` | `rewards`, `retry-reward`, `throttles`, `clear-throttle`, `audit` |
| `/shutdown` | `shutdown` |
| `/purge <count>` | 채널 메시지 정리(대시보드 제외) |

`duration`은 `90`, `30m`, `12h`, `7d`, `2w` 또는 `30분`, `12시간`, `7일`, `2주` 형식입니다. 계정 삭제, 전원 퇴장, 점검+퇴장, 서버 종료는 확인 버튼을 한 번 더 누릅니다.

## 설치

1. [Discord Developer Portal](https://discord.com/developers/applications)에서 앱을 만들고 **Bot** 탭에서 토큰을 발급합니다. **Public Bot은 끄세요.** Privileged Gateway Intents는 필요 없습니다.
2. `<APP_ID>`를 앱 ID로 바꿔 서버에 초대합니다.
   ```
   https://discord.com/oauth2/authorize?client_id=<APP_ID>&scope=bot+applications.commands&permissions=326417591296
   ```
3. Discord 개발자 모드를 켜고 서버 ID를 복사합니다.
4. `.env`에 `DISCORD_TOKEN`, `DISCORD_GUILD_ID`를 적고 서버를 다시 시작합니다. compose 없이 실행하면 `P5099_DISCORD_TOKEN`, `P5099_DISCORD_GUILD_ID`를 씁니다. 토큰이 있는데 서버 ID가 없으면 서버가 시작하지 않습니다.
5. 대시보드를 올릴 채널에서 `/dashboard create`를 실행하고, 운영 기록 채널은 `/settings`로 고릅니다.

| 변수 | 기본값 | 설명 |
|---|---|---|
| `P5099_DISCORD_TOKEN` | 없음(봇 꺼짐) | 봇 토큰 |
| `P5099_DISCORD_GUILD_ID` | 토큰이 있으면 필수 | 봇을 쓸 Discord 서버. 다른 서버에서는 나갑니다. |
| `P5099_DISCORD_STATE_FILE` | `<--profile-root>/Discord.json` | 대시보드 위치와 `/settings` 값 |

`/settings`에서 알림 채널, 접속·퇴장 알림(기본 꺼짐), 운영 기록 채널, 갱신 간격(10~300초, 기본 30초)을 바꿉니다. 봇을 빼고 빌드하려면 `cargo build -p p5099-server --no-default-features`를 씁니다.

## 권한

| 누가 | 쓸 수 있는 것 |
|---|---|
| 모든 멤버 | `/status`, `/online`, `/rooms`, `/ping`, `/help`, 대시보드 조회 버튼. IP는 보이지 않습니다. |
| **서버 관리** 권한 | 나머지 명령, 운영 패널, 계정 카드, 입력 창 |
| 메시지 관리 권한 | `/purge` |

명령을 다른 역할에 열어도 서버 관리 권한이 없으면 봇이 거절합니다. 운영자 역할에는 서버 관리 권한을 주세요.

## 보안

- 봇은 서버의 모든 운영 권한을 서버 관리 권한이 있는 멤버에게 엽니다. `/players`, `/account info`는 IP를 보여 줍니다.
- 봇이 서버 프로세스 안에서 돌기 때문에 봇의 결함은 `Accounts.json`과 프로필 전체에 영향을 줄 수 있습니다. **토큰이 새면 바로 재발급하세요.**
- 봇으로 한 변경은 `audit`과 서버 로그에 `discord:<사용자 이름> (<사용자 ID>)`로 남습니다.

## 개발 메모

- 서버 쪽 연결은 `crates/p5099-server/src/bin/p5099-server/discord.rs`입니다.
- components v2 때문에 serenity `next` 브랜치를 커밋 단위로 고정해 씁니다. 0.12.x로는 v2 버튼을 해석하지 못하고, `next`가 Rust 1.95를 요구합니다.
- 실행 중인 서버로 모든 콘솔 호출 확인(테스트 계정을 만들고 지웁니다):
  ```bash
  P5099_ADMIN_ADDRESS=127.0.0.1:39318 P5099_ADMIN_TOKEN=... cargo test -p p5099-discord -- --ignored
  ```
