# P5099 Rust 서버

[변경 기록](CHANGELOG.md)

한국 카트라이더 P5099 클라이언트를 위한 독립 Rust 서버와 실행기입니다. 목표는 친구들끼리 방 생성·입장, 경기 시작, 주행, 결과·시상식, 방 복귀까지의 멀티플레이 사이클을 안정적으로 지원하는 것입니다. 모든 상용 서비스 기능을 재현하지는 않으며 Lucci, 보너스 아이템, 팀 플래그와 일부 이벤트·소셜·상점 기능은 범위 밖이거나 제한적으로 응답합니다.

| 실행 파일 | 대상 | 역할 |
|---|---|---|
| `p5099-launcher` | 플레이어 | 계정 로그인, 클라이언트 다운로드·업데이트, 게임 실행, Discord 상태 표시 |
| `p5099-server` | 운영자 | 게임 서버. Discord 운영 봇이 함께 돕니다. |
| `p5099-assets` | 운영자 | 클라이언트 파일 목록 생성(`client-manifest`), 외부 트랙·자산 가져오기 |
| `p5099-admin` | 운영자 | 운영 콘솔(RCON) 명령줄 도구 |

## 서버 실행

`docker-compose.yml`로 실행하며 설정은 같은 폴더의 `.env`에서 읽습니다.

```bash
cp .env.example .env   # 값을 채웁니다
docker compose up -d
```

| `.env` 항목 | 필수 | 설명 |
|---|---|---|
| `BIND` | 예 | 서버가 포트를 열 로컬 주소 |
| `ADVERTISE` | 예 | 로그인·채널 패킷에 넣을 공인 IPv4. 도메인과 `0.0.0.0`은 안 됩니다. |
| `REQUIRE_ACCOUNT` | 예 | 실행기 계정 로그인 필수 여부 |
| `PROFILE_DIR`, `DATA_DIR`, `DISTRIBUTION_DIR` | 예 | 프로필, 클라이언트 `Data`, 배포 폴더([클라이언트 다운로드](docs/LAUNCHER_ACCOUNTS.md#클라이언트-다운로드)) |
| `DISABLE_ACCOUNT_REGISTRATION` | | 가입 막기 |
| `RCON_TOKEN`, `RCON_HOST` | | [운영 콘솔](docs/ADMIN_CONSOLE.md) 토큰(16자 이상)과 바인드 주소(기본 `127.0.0.1`) |
| `CONFIGURED_PORT` | | 기준 포트(기본 `39311`) |
| `DISCORD_TOKEN`, `DISCORD_GUILD_ID` | | [Discord 운영 봇](docs/DISCORD_BOT.md) |

필수 항목이 빠지면 compose가 시작하지 않습니다. compose가 이 이름들을 서버의 `P5099_*` 환경 변수로 옮겨 줍니다.

직접 빌드해 실행할 수도 있습니다(Rust 1.95 이상). 모든 옵션은 `P5099_<옵션 이름>` 환경 변수로도 줄 수 있고 명령줄이 우선합니다. 전체 옵션은 `p5099-server --help`에서 확인하세요.

```bash
cargo build --release -p p5099-server
./target/release/p5099-server --bind 0.0.0.0 --advertise <공인 IPv4> \
  --client-dir ./Data --distribution-dir ./Distribution --require-account
```

- `--client-dir`: 클라이언트 루트, `Profile` 또는 `Data` 폴더. 서버는 `kart.rho`, `item.rho`, `track_common.rho`, RHO5를 읽기 전용으로 직접 해석하며, 읽지 못하면 시작하지 않습니다.
- 계정 없이 LAN에서 테스트할 때는 `--require-account` 대신 `--allow-remote-profile-creation`을 씁니다.
- Windows Server 2012용 빌드는 [WINDOWS_SERVER_2012.md](WINDOWS_SERVER_2012.md)를 참고하세요.

## 포트

기준 포트 `39311` 기준입니다. 방화벽(클라우드 보안 목록 포함)에서 열어야 하며, **운영 콘솔 `39318`은 외부에 열지 말고 SSH 터널로 접근하세요.**

| 용도 | 프로토콜 | 포트 |
|---|---:|---:|
| 게임 | UDP | 39311 |
| 로그인 / P2P relay | TCP / UDP | 39312 |
| 메신저 | TCP | 39313 |
| XUN 보조 채널 | TCP | 39314 |
| 친구 브리지 (`nmcogame.dll`) | TCP | 39315 |
| 계정 서비스 | TCP | 39316 |
| 클라이언트 다운로드 (HTTP) | TCP | 39317 |
| 운영 콘솔 (RCON, 선택) | TCP | 39318 |

## 플레이어

플레이어는 설치기(Windows `P5099-Setup.exe`, macOS `P5099.dmg`)로 실행기를 설치하고, 서버 주소를 입력해 가입·로그인한 뒤 `게임 시작`을 누릅니다. 실행기가 클라이언트를 받아 최신 상태로 맞추고 게임을 실행합니다. macOS는 [macOS에서 플레이하기](docs/MACOS.md)를 참고하세요.

## 게임 기능

- **방 제목 물리 토큰**: 방 제목에 독립된 `S0`~`S8`(대소문자 무관)을 넣으면 다음 경기부터 해당 물리 프리셋을 씁니다. `S10`, `TESTS1ROOM`은 인식하지 않습니다. 토큰이 없으면 일반 채널은 S7, 무한부스터 채널은 S4입니다.
- **팀 배치와 출발 순서**: 새 레이서는 인원이 적은 팀(같으면 블루)에 들어갑니다. 직전 경기 순위가 다음 출발 순서가 되고, 방장은 직전 경기 최고 순위 인간 레이서(팀전은 승리팀 안에서)로 바뀝니다.
- **카트 강화·플로터**: 재료와 재화는 소모하지 않고 성공률은 100%입니다. 결과는 닉네임별 `LevelData.json`, `TuneData.json`에 저장되고 실제 경기 물리에 반영됩니다.
- **관리자 역할**: `Accounts.json`에서 `"Admin": true`인 계정만 `launcher.xml`의 `<pmap>`으로 옵저버(718, 방장 권한 유지)·익명 리그(1798) 역할을 요청할 수 있습니다.
- **AI 난이도**: 경기 시작 패킷에 AI마다 float 6개가 들어갑니다. 기본값은 스피드전 `[0.7, 2400, 2950, 1.5, 1000, 1500]`, 아이템전 `[0.6, 2400, 2950, 1.5, 1000, 1500]`입니다. 앞의 두 값의 곱이 목표 속도, 세 번째가 부스터 시간(ms), 네 번째가 부스터 가속 배율이고, 나머지 둘은 클라이언트가 읽지 않습니다.

## 외부 트랙·자산 가져오기

`p5099-assets`로 다른 클라이언트의 트랙(`discover-tracks`, `import-tracks-data-raw`)과 카트·캐릭터·펫·플라잉펫(`list-compatible-assets`, `import-assets-data-raw`)을 현재 `DataRaw`에 추가합니다.

- 기존 파일과 내용이 다르면 덮어쓰지 않고 중단합니다. 작업물과 원본 백업은 `.p5099-track-import`, `.p5099-asset-import`, `*.pristine.bak`에 남습니다.
- 가져올 수 있는 것: 일반 카트 51종, XUN 시험 카트 100종(`defaultExceedType` 1~4), 캐릭터 73종, 펫 24종, 플라잉펫 47종. 신규 테마 트랙은 검증된 `fengshen`만 됩니다.
- XUN 카트는 정확한 빌드용 [XUN 사이드카](native/p5099-xun-sidecar/README.md)가 필요합니다.
- **적용 후 할 일**: 서버를 다시 시작하고, 바뀐 클라이언트 파일을 배포 폴더에 복사한 뒤 파일 목록을 다시 만들고 서명합니다.

## 로그

서버는 실행할 때마다 실행 파일 옆 `logs/p5099-server-<timestamp>-<pid>.log`를 만듭니다. 패킷 내용(hex)은 기본으로 남기지 않으며 `--log-packet-payloads`로 켭니다. 클라이언트 크래시를 조사할 때는 서버 로그와 클라이언트 `logs` 폴더를 함께 보관하세요.

## 개발

```bash
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

실제 클라이언트 데이터 smoke test는 `P5099_CLIENT_DATA_DIR=<Data 폴더>`를 지정해 실행합니다. 포팅 현황과 지켜야 할 불변 조건은 [PORTING.md](PORTING.md), 클라이언트 상태 모델은 [CLIENT_PROTOCOL_FSM.md](CLIENT_PROTOCOL_FSM.md), 친구창 DLL 규격은 [docs/NMCO_BRIDGE.md](docs/NMCO_BRIDGE.md)에 있습니다.

워크스페이스는 `unsafe_code = "forbid"`를 적용합니다. 라이선스는 `AFL-3.0`입니다.
