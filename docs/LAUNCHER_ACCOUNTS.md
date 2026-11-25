# 플레이어 실행기와 계정

공개 서버에 플레이어가 닉네임과 비밀번호로 접속하는 구조입니다.

| 구성 요소 | 역할 |
|---|---|
| `p5099-launcher` (설치 후 `p5099`) | 서버 주소(`호스트` 또는 `호스트:기준 포트`), 로그인, 가입, 비밀번호 변경. 처음 서버 주소는 빌드할 때 `P5099_SERVER_HOST`로 정합니다. |
| 계정 서비스 (TCP `39316`) | 가입·로그인·비밀번호 변경, 실행 티켓 발급 |
| `p5099-server --require-account` | 게임 로그인에 실행 티켓이 있어야 입장 허용 |
| `nmcogame.dll` | 친구 브리지(`39315`)에 같은 티켓을 보냅니다. |

## 설치기와 실행기

| 플랫폼 | 설치기 | 설치 위치 | 게임 파일 |
|---|---|---|---|
| Windows | `P5099-Setup.exe` (`installer/windows/p5099-setup.iss`) | `C:\Program Files (x86)\p5099` | `%ProgramData%\p5099\Game\` |
| macOS | `P5099.dmg` (`installer/macos/make-dmg.sh`) | 응용 프로그램 폴더 | `~/Library/p5099/Game` |

- Windows 설치기는 게임 폴더에 모든 사용자 쓰기 권한을 주므로 실행기는 관리자 권한 없이 게임을 받습니다. 제거하면 기본 위치의 게임 파일도 지웁니다.
- 설치 위치는 실행기 설정(⚙ → `설치 위치`)에서 바꿀 수 있습니다. 이전 위치의 파일은 지우지 않습니다.
- **게임 경로에 띄어쓰기를 쓰지 마세요.** 오래된 클라이언트와 Nexon 모듈이 경로를 잘라 읽습니다. 그래서 macOS는 `Application Support` 대신 `~/Library/p5099`를 씁니다.
- macOS 앱은 서명·공증하지 않아 처음에는 Finder에서 우클릭 → 열기로 엽니다. CrossOver 또는 Sikarugir가 필요합니다([macOS에서 플레이하기](MACOS.md)).

## Discord 상태 표시

실행기가 15초마다 계정 서비스에 라이더 위치를 물어 Discord에 `로비` 또는 `아이템 팀전 · 대기 중`/방 이름/인원을 표시합니다. 빌드할 때 `P5099_DISCORD_APPLICATION_ID`가 없으면 표시하지 않습니다. 설정 → `Discord` → `게임 상태 표시`로 끌 수 있고, "참가 요청"은 지원하지 않습니다.

## 클라이언트 다운로드

서버는 `--distribution-dir` 폴더를 TCP `39317`에서 HTTP로 제공하고, 실행기는 `게임 시작` 때 여기서 클라이언트를 받아 맞춥니다.

- 배포 폴더 구성: `windows/`, `macos/`(macOS·Linux Wine용). 실행기 업데이트는 서버가 아니라 GitHub Release에서 받습니다.
- 각 폴더의 `p5099-manifest.txt`(경로·크기·SHA-256)와 `p5099-manifest.sig`(Ed25519 서명). 전송이 평문 HTTP라서 **실행기는 `P5099_RELEASE_PUBLIC_KEY`로 서명이 확인된 목록만 믿습니다.**
- 목록의 `revision`(만든 시각)보다 오래된 목록은 거부합니다. 목록은 시계가 맞는 컴퓨터에서 만드세요.
- 없는 파일과 내용이 다른 파일만 받습니다. `KartRider.pin`처럼 실행기가 수정하는 파일은 없을 때만 받습니다. `KartRider.pin`은 두 폴더 모두에 꼭 있어야 합니다.

### 클라이언트를 올리거나 갱신할 때

1. 플랫폼별 클라이언트 폴더에 `KartRider.pin`을 넣고, `nmcogame.dll`을 친구창 브리지(`native/p5099-nmco/out/nmcogame.dll`)로 교체합니다. 원본에 다른 `dinput8.dll`이 있으면 뺍니다.
2. 폴더마다 목록을 만들고 서명합니다. 서명이 목록 옆 `p5099-manifest.sig`에 쓰이므로 두 목록은 다른 폴더에서 만듭니다.
   ```bash
   p5099-assets client-manifest --client-dir <windows용 클라이언트> --out windows-manifest.txt
   p5099-assets client-manifest --client-dir <macos용 클라이언트> --out macos-manifest.txt
   installer/launcher-release.sh sign-manifest launcher-signing-key.pem windows-manifest.txt
   installer/launcher-release.sh sign-manifest launcher-signing-key.pem macos-manifest.txt
   ```
   `Profile/`, `logs/`, `KartRider.xml`, `p5099-xun-session.ini`, `*.log`, `*.bak`, `*.pristine*`, 점으로 시작하는 폴더는 제외됩니다.
3. **바뀐 파일을 먼저 복사하고 목록과 서명은 마지막에 넣습니다.** 그래야 실행기가 아직 없는 파일을 요청하지 않습니다. 서버는 재시작할 필요가 없습니다.

## 실행기 업데이트

실행기는 시작할 때 이 저장소의 최신 GitHub Release에 올라간 `latest.txt`(첫 줄 서명, 나머지 JSON, `releases/latest/download/latest.txt`)를 확인해 새 버전이 있으면 `업데이트 후 재시작` 버튼을 보여 줍니다. 버전이 `min_version`보다 낮으면 업데이트 전까지 로그인을 막습니다. 새 실행기 파일(`p5099.exe`, `p5099.app.zip`)도 같은 Release에서 받고, 서명된 `latest.txt`의 크기·SHA-256과 맞을 때만 교체합니다.

어느 저장소를 볼지는 빌드할 때 `P5099_RELEASE_REPO`(`owner/repo`)로 정합니다. CI는 자동으로 빌드한 저장소를 넣고, 이 값이나 공개키 없이 직접 빌드한 실행기는 업데이트를 확인하지 않습니다. 초안(draft)·시험판(pre-release)은 `latest`에 잡히지 않습니다.

### 처음 한 번: 서명 키 만들기

```bash
installer/launcher-release.sh keygen launcher-signing-key.pem
```

공개키는 GitHub 저장소 변수 `P5099_RELEASE_PUBLIC_KEY`에, 개인키는 secret `LAUNCHER_SIGNING_KEY`에 넣고 따로 보관합니다. 공개키가 비어 있으면 실행기는 클라이언트를 받지 못합니다. **키를 잃어버리거나 바꾸면 배포된 실행기는 업데이트를 받지 못해 새 설치기로 다시 설치해야 합니다.**

### 릴리스할 때

1. `Cargo.toml`의 `version`을 올리고 같은 태그(`v1.1.0`)를 푸시합니다. 다르면 빌드가 실패합니다.
2. CI가 서명한 `latest.txt`, `p5099.exe`, `p5099.app.zip`을 설치기와 함께 그 태그의 GitHub Release에 올립니다. 서버에 따로 복사할 것은 없습니다. 서명 키 secret이 없으면 Release가 만들어지지 않습니다.
3. 강제 업데이트가 필요하면 저장소 변수 `LAUNCHER_MIN_VERSION`을 새 버전으로 바꿉니다.

## 로그인 흐름

1. 실행기가 비밀번호를 PBKDF2-HMAC-SHA256(100,000회, 닉네임별 salt)으로 32바이트 키로 바꿉니다. **비밀번호 자체는 서버로 보내지 않습니다.**
2. 가입은 이 키를 한 번 보내고, 서버는 `Profile/Accounts.json`에 키만 저장합니다.
3. 로그인은 서버가 준 nonce에 대한 HMAC 증명을 보내고 32바이트 실행 티켓을 받습니다.
4. 실행기는 티켓을 `Profile/kr/launcher.xml`과 `p5099-xun-session.ini`에 쓰고 게임을 실행합니다. 서버는 그 닉네임의 가장 최근 티켓만 받습니다.

티켓은 메모리에만 있고 24시간 유효합니다. **서버를 재시작하면 모두 사라지므로 다시 로그인해야 합니다.**

`로그인 유지`를 켜면 비밀번호가 아니라 위의 키를 시스템 자격 증명 저장소(Windows 자격 증명 관리자, macOS 키체인)에 저장합니다. 키는 서버마다 같으므로 **저장한 서버에만 보냅니다.** Linux 실행기는 저장하지 않습니다.

## 와이어 형식

한 줄에 UTF-8 메시지 하나, 필드는 탭으로 구분합니다. 연결당 최대 8개 요청, 30초 제한입니다. 구현은 `crates/p5099-core/src/account_protocol.rs`입니다.

| 요청 | 응답 |
|---|---|
| `REGISTER\t<닉네임>\t<키 hex>` | `REGISTERED\t<닉네임>` |
| `CHALLENGE\t<닉네임>` | `NONCE\t<hex>` |
| `LOGIN\t<닉네임>\t<HMAC(키, "P5099-login-proof-v1\0" ‖ nonce) hex>` | `TICKET\t<닉네임>\t<티켓 hex>` |
| `PASSWORD\t<닉네임>\t<증명 hex>\t<새 키 hex>` | `PASSWORD_CHANGED` |
| `PRESENCE\t<닉네임>\t<티켓>` | 라이더 위치(`offline`/`lobby`/`room`) |

실패는 `ERROR\t<코드>`: `exists`, `invalid_nickname`, `bad_credentials`, `rate_limited`, `registration_closed`, `bad_request`, `unavailable`.

## 운영

- 방화벽에서 TCP `39315`, `39316`, `39317`을 엽니다.
- 관리자: `Accounts.json`에서 `"Admin": true`로 바꾸고 재시작하거나 `p5099-admin admin <닉네임>`을 씁니다.
- **기존 프로필은 그 닉네임으로 먼저 가입한 사람이 이어받습니다.** 공개 전에 기존 유저가 각자 가입하게 하세요.
- 제한: 같은 IP에서 10분에 로그인 실패 10회, 1시간에 가입 5회.

## 보안상 한계

- 전송 구간을 암호화(TLS)하지 않습니다. 가입 순간을 도청하면 저장 키를 알아 이 서버에 그 계정으로 로그인할 수 있습니다.
- 로그인 유지를 켠 PC의 사용자 계정에 접근할 수 있는 사람은 그 계정으로 로그인할 수 있습니다. 공용 PC에서는 켜지 마세요.
- 게임 트래픽을 가로채면 다음 로그인 전까지 실행 티켓을 재사용할 수 있습니다.
