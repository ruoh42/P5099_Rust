# macOS에서 플레이하기 (CrossOver / Sikarugir)

카트라이더는 Windows 게임이므로 macOS에서는 CrossOver 또는 Sikarugir
안에서 실행합니다. 실행기(`p5099.app`)가 클라이언트 다운로드(`~/Library/p5099/Game`)와
실행을 모두 처리하므로 둘 중 하나만 준비하면 됩니다.

| | CrossOver (유료) | Sikarugir (무료) |
|---|---|---|
| 준비 | CrossOver 설치 | Sikarugir Creator로 wrapper 1개 만들기 |
| 실행기가 하는 일 | 보틀이 없으면 처음 시작할 때 만들고 그 안에서 실행 | wrapper의 Wine과 prefix로 게임 폴더의 `KartRider.exe`를 직접 실행 |

실행 방식이 `자동`이면 CrossOver가 설치되어 있을 때 CrossOver를, 없으면
Sikarugir wrapper를 사용합니다. 로그인 화면의 클라이언트 상태 아래에
`실행 환경 · …`으로 현재 선택이 표시됩니다.

## CrossOver

1. CrossOver를 `/Applications`에 설치합니다.
2. 실행기에서 `게임 시작`을 누릅니다. 보틀 `p5099`가 없으면
   실행기가 만듭니다(처음 한 번, 1~2분). 게임 파일은 보틀 안이 아니라
   실행기의 게임 폴더에 그대로 있습니다.

이미 만들어 둔 보틀을 쓰려면 설정(⚙) → `CrossOver 보틀`의 목록에서
고릅니다. CrossOver를 다른 위치에 설치했다면 실행 방식을 `CrossOver`로 두고 `CrossOver`에
`CrossOver.app/Contents/SharedSupport/CrossOver/bin/wine` 경로를 지정합니다.

## Sikarugir

1. Sikarugir Creator에서 엔진을 받고 wrapper를 만듭니다. 이름은 자유이며
   기본 저장 위치(`~/Applications/Sikarugir`)에 두면 실행기가 자동으로
   찾습니다. wrapper에 게임을 설치하거나 실행 파일·`exec*.bat`를 설정할
   필요는 없습니다.
2. 실행기의 설정(⚙) → `실행 방식`을 `Sikarugir`(또는 `자동`)로 두고,
   `Sikarugir` 줄에서 만든 wrapper를 고릅니다. 다른 위치에 있다면
   `찾아보기…`로 `.app`을 지정합니다. 새로 만든 wrapper가 목록에 없으면
   ⟳(새로 고침) 버튼을 누릅니다.
3. `게임 시작`을 누릅니다.

실행기는 wrapper의 `Contents/SharedSupport/wine/bin/wine`을
`WINEPREFIX=Contents/SharedSupport/prefix`로 실행하고, 작업 디렉터리를 게임
폴더로 맞춥니다. wrapper 앱을 직접 열지 않으므로 wrapper의 프로그램 설정은
쓰이지 않습니다. prefix에 저장되는 설정(Winetricks, DLL 재정의, 레지스트리)은
그대로 적용됩니다.

## 문제 해결

- **`실행 환경`이 빨간 글씨**: CrossOver도 Sikarugir wrapper도 찾지 못한
  경우입니다. 위 둘 중 하나를 준비합니다. wrapper는 `.app` 안에
  `Contents/SharedSupport/wine/bin/wine`과 초기화된 `prefix`가 있어야 합니다.
- **소리 문제로 멈춤**: Wine 로그에 `mmdevapi` assertion이나 CoreAudio 채널
  레이아웃 오류가 있고 게임이 멈출 때만, 해당 보틀·prefix의 레지스트리
  `HKEY_CURRENT_USER\Software\Wine\DllOverrides`에
  `"winecoreaudio.drv"="disabled"`를 추가합니다(CrossOver: 보틀의
  `regedit`, Sikarugir: wrapper의 Wine으로 `regedit`). 게임 소리가 나지 않을
  수 있습니다.
- **처음 실행 확인 창**: macOS 앱은 서명·공증하지 않았으므로 처음 한 번은
  Finder에서 `p5099.app`을 우클릭 → 열기로 엽니다.
- 서버 쪽에서는 P5099 로그에 `PqLogin` 수신과 `PrLogin` 송신이 보이면
  로그인까지 성공한 것입니다.
