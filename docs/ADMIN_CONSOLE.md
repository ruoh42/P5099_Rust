# 운영 콘솔 (RCON)

`p5099-server`를 관리하는 [Source RCON](https://developer.valvesoftware.com/wiki/Source_RCON_Protocol) 콘솔입니다. 기준 포트 + 7(기본 TCP `39318`)에서 열리며, 토큰이 없으면 꺼져 있습니다. 토큰이 RCON 비밀번호이므로 `p5099-admin` 외에 `mcrcon`, `rcon-cli` 같은 일반 RCON 클라이언트로도 접속할 수 있습니다.

## 켜기

```bash
export P5099_ADMIN_TOKEN="$(openssl rand -hex 32)"   # 16자 이상
p5099-server ...   # 또는 --admin-token-file <파일>
```

| 설정 | 기본값 | 설명 |
|---|---|---|
| `P5099_ADMIN_TOKEN` (compose: `RCON_TOKEN`) | 없음 | 콘솔 토큰. 없으면 콘솔이 꺼집니다. |
| `--admin-token-file` | 없음 | 토큰을 파일에서 읽습니다(환경 변수보다 우선). |
| `--admin-bind` (compose: `RCON_HOST`) | `127.0.0.1` | 콘솔을 열 주소. `--bind`와 별개입니다. |

## 보안

- **RCON은 암호화하지 않습니다.** 콘솔은 loopback에 두고 원격에서는 SSH 터널을 쓰세요: `ssh -N -L 39318:127.0.0.1:39318 운영서버`
- 한 주소에서 10분 안에 토큰을 10번 틀리면 그 주소는 맞는 토큰이어도 거부됩니다.
- 토큰은 명령줄 인자로 받지 않습니다. 모든 변경 작업은 서버 로그(`p5099_admin` 대상)에 남습니다.

## `p5099-admin` 명령

```bash
export P5099_ADMIN_TOKEN=...
p5099-admin health                        # 콘솔 응답과 토큰 확인
p5099-admin status                        # 가동 시간, 접속자, 방, 계정 수, 점검 모드, 보상 복구 상태
p5099-admin players                       # 접속자
p5099-admin rooms                         # 방
p5099-admin accounts                      # 계정 목록
p5099-admin info <닉네임>                 # 계정 상세

# 계정
p5099-admin create <닉네임> [--admin]     # 가입이 닫혀 있어도 가능, 비밀번호는 표준 입력
p5099-admin delete <닉네임> --yes         # 계정 삭제 + 퇴장 (프로필 파일은 남음)
p5099-admin ban <닉네임> --reason "..." [--duration 7d]
p5099-admin unban <닉네임>
p5099-admin admin <닉네임> / unadmin <닉네임>
p5099-admin password <닉네임>             # 새 비밀번호는 표준 입력

# 접속 관리
p5099-admin kick <닉네임>
p5099-admin kick-ip <주소>
p5099-admin kick-room <방 번호>
p5099-admin kick-all [--keep-admins]
p5099-admin ban-ip <주소> --reason "..." / unban-ip <주소> / bans

# 서버 운영
p5099-admin maintenance [on [--kick] | off]
p5099-admin registration open|close       # 재시작 전까지
p5099-admin rewards                       # 저장 중·저장 실패 경기 보상
p5099-admin retry-reward [<방> <경기 번호>]
p5099-admin throttles / clear-throttle [<주소>]
p5099-admin audit [--limit 50]
p5099-admin shutdown
```

공통 옵션은 `--host`, `--port`(기본 `39318`), `--token-file`, `--json`입니다. `--duration`은 `90`(초), `30m`, `12h`, `7d` 형식입니다. `-`로 시작하는 값은 `--reason=-값`처럼 씁니다.

## RCON 명령

일반 RCON 클라이언트로 보낼 때의 명령입니다. 공백이 들어간 값은 따옴표로 감쌉니다. 응답은 JSON이고 실패하면 `{"error": "..."}`입니다.

| 명령 | 설명 |
|---|---|
| `help`, `health`, `status`, `players`, `rooms`, `accounts`, `info <닉네임>` | 조회 |
| `create <닉네임> <비밀번호> [--admin]`, `delete <닉네임>` | 계정 생성·삭제 |
| `admin` / `unadmin <닉네임>` | 관리자 지정·해제 |
| `ban <닉네임> [--reason <사유>] [--duration <기간>]`, `unban <닉네임>` | 정지·해제 |
| `password <닉네임> <비밀번호>` | 비밀번호 재설정, 기존 티켓 폐기 |
| `bans`, `ban-ip <주소> [--reason <사유>]`, `unban-ip <주소>` | IP 차단 |
| `kick <닉네임>`, `kick-ip <주소>`, `kick-room <방 번호>`, `kick-all [--keep-admins]` | 강제 퇴장 |
| `registration open\|close`, `maintenance [on\|off] [--kick]` | 가입·점검 |
| `rewards`, `retry-reward [<방> <경기 번호>]` | 보상 저장 실패 처리 |
| `throttles`, `clear-throttle [<주소>]` | 시도 제한 |
| `audit [--limit <n>]` | 재시작 이후 운영 기록(최대 256건) |
| `shutdown` | 정상 종료 |

```bash
mcrcon -H 127.0.0.1 -P 39318 -p "$P5099_ADMIN_TOKEN" 'ban "라이더 이름" --reason abuse --duration 1d'
```

연결은 동시에 16개, 5분 동안 입력이 없으면 끊습니다. 4086바이트를 넘는 응답은 여러 패킷으로 나뉩니다.

## 동작 메모

- 정지된 계정은 실행기 로그인과 이미 받은 실행 티켓이 모두 거부됩니다. 기간이 지나면 자동으로 풀립니다.
- IP 차단은 `Accounts.json`에 저장되어 재시작 후에도 유지됩니다.
- 점검 모드, 가입 열기/닫기, 최근 로그인 정보, 감사 기록은 재시작하면 초기화됩니다.
- 점검 중에는 관리자만 로그인할 수 있습니다. 계정 없이 운영하는 서버에서는 모든 게임 로그인이 거부됩니다.
- 관리자 지정은 다음 게임 로그인부터 적용됩니다.
- 계정을 삭제해도 프로필 파일은 남고, 같은 닉네임으로 다시 가입하면 이어서 씁니다.
- **재시도 가능한 보상 저장 실패가 남아 있으면 정상 종료가 멈춥니다.** `retry-reward`로 해결하거나 `shutdown`을 한 번 더 보내 강제 종료합니다.
- 서버 전체 공지는 아직 없습니다.
