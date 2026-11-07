# downcop · 다운로드단속반

Windows 10 / 11에서 **시스템 전체의 다운로드(인바운드)·업로드(아웃바운드) 트래픽을 원클릭으로 차단/해제하거나 속도를 제한**하는 가벼운 트레이 유틸리티입니다.

- 방향별(다운로드/업로드) **차단** 토글과 **속도 제한**(KB/s · MB/s)
- **전체 차단 / 전체 해제** 원클릭 버튼과 트레이 메뉴
- 현재 송수신 속도와 최근 60초 미니 그래프
- **로컬 네트워크(LAN) 제외** 옵션 (공유기·NAS·프린터 등은 그대로 통신). 루프백(127.0.0.1)은 항상 제외
- **윈도우 시작 시 자동 실행**과 **실행 즉시 마지막 상태 적용**을 별도 설정으로 제공
- 한국어 / English 전환
- 트레이 아이콘 색으로 상태 표시: 초록(정상), 주황(제한 중), 빨강(차단 중)
- Rust + [Slint](https://slint.dev) 단일 exe(약 8MB), 런타임 설치 불필요

## 설치와 실행

1. [Actions](../../actions) 빌드 아티팩트나 [Releases](../../releases)에서 `downcop-windows-x64.zip`을 받습니다.
2. 아무 폴더에나 압축을 풉니다. `downcop.exe`, `WinDivert.dll`, `WinDivert64.sys`는 **같은 폴더**에 있어야 합니다.
3. `downcop.exe`를 실행합니다. 패킷 드라이버를 쓰기 때문에 **관리자 권한(UAC 확인)** 이 필요합니다.

창을 닫으면 트레이로 숨겨집니다. 트레이 아이콘을 왼쪽 클릭하면 창이 열리거나 숨겨지고, 오른쪽 클릭하면 메뉴가 뜹니다. 완전히 끄려면 트레이 메뉴에서 **종료**를 누릅니다. 이미 실행 중일 때 exe를 다시 실행하면 기존 창이 앞으로 나옵니다.

## 설정

| 설정 | 설명 |
|---|---|
| 로컬 네트워크(LAN)는 제외 | 사설 IP(10/8, 172.16/12, 192.168/16, 169.254/16), IPv6 링크로컬·ULA(fe80::/10, fc00::/7), 멀티캐스트·브로드캐스트는 차단이나 제한을 하지 않습니다. 기본값은 켜짐입니다. |
| 윈도우 시작 시 자동 실행 | 로그인할 때 트레이로 조용히 시작합니다. 작업 스케줄러에 `downcop` 작업(가장 높은 권한)을 등록하므로 매번 UAC 창이 뜨지 않습니다. |
| 실행하면 마지막 상태 바로 적용 | 켜면 시작하자마자 마지막 차단·제한 상태를 다시 적용합니다. 끄면 항상 "정상(통과)" 상태로 시작하고, 입력해 둔 제한 값만 유지됩니다. |

두 설정은 서로 독립입니다. 예를 들어 "자동 실행만 하고 차단은 직접 켜기"나 "수동 실행이지만 켜자마자 차단" 같은 조합이 가능합니다.

설정 파일은 `%APPDATA%\downcop\config.json`, 로그는 `%APPDATA%\downcop\downcop.log`에 저장됩니다.

## 동작 원리

- **[WinDivert](https://reqrypt.org/windivert.html) 2.2**(서명된 WFP 드라이버)로 IP 패킷을 사용자 모드에서 가로챕니다.
  - Windows 방화벽은 연결 상태를 추적하므로, 내가 먼저 연결한 쪽으로 돌아오는 응답(=다운로드)을 막지 못합니다.
  - QoS 정책은 업로드만 제한할 수 있습니다.
- **차단**: 해당 방향의 패킷을 재주입하지 않고 버립니다.
- **속도 제한**: 방향별 토큰 버킷으로 패킷을 큐에 잡아 두었다가 설정 속도로 내보냅니다. 큐(약 1초 분량, 최대 4MB)가 넘치면 버리고, TCP가 알아서 속도를 줄입니다.
- 개입이 필요한 방향만 캡처합니다. 둘 다 "정상"이면 드라이버 핸들을 닫아 오버헤드가 없습니다.
- **안전장치**: downcop이 종료되거나 비정상 종료되면 핸들이 닫히면서 모든 트래픽이 즉시 원래대로 흐릅니다. 방화벽 규칙처럼 차단 상태가 남지 않습니다.
- 화면의 속도는 물리 네트워크 어댑터의 송수신 합계입니다(1 KB = 1024 B).

## 문제 해결

| 증상 | 해결 |
|---|---|
| "WinDivert.dll과 WinDivert64.sys를 … 같은 폴더에 두세요" | zip의 세 파일을 같은 폴더에 둡니다. |
| "관리자 권한으로 실행해야 합니다" | 우클릭 → 관리자 권한으로 실행 |
| 드라이버를 불러오지 못함(577, 1275 등) | 일부 백신은 WinDivert를 위험 소프트웨어로 오탐합니다. 예외로 등록하거나, Windows 보안의 '메모리 무결성' 설정을 확인하세요. |
| 다른 WinDivert 버전이 사용 중 | WinDivert를 쓰는 다른 프로그램(일부 VPN, 게임 가속기 등)을 끄거나 재부팅합니다. |

## 빌드

Rust 1.88 이상이 필요합니다.

```bash
# Windows (MSVC)
cargo build --release
# target\release\downcop.exe 옆에 WinDivert-2.2.2-A\x64\WinDivert.dll, WinDivert64.sys 복사
```

```bash
# Linux에서 Windows용 크로스 빌드 (mingw-w64 필요)
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu
```

```bash
# 테스트 (어느 OS든 가능). Windows에서 테스트 exe 옆에 WinDivert 파일이 있으면
# 생성된 필터를 실제 WinDivert 파서와 평가기로도 검증합니다.
cargo test
```

Windows가 아닌 OS에서 `cargo run`을 하면 **미리보기 모드**로 실행됩니다. UI만 동작하고 트래픽은 제어하지 않습니다.
CI(`.github/workflows/build.yml`)는 Windows에서 fmt, clippy, test, release 빌드를 돌린 뒤 WinDivert를 동봉한 zip을 만들고, `v*` 태그를 push하거나 Actions에서 **build** 워크플로를 수동 실행하며 `release_tag`(예: `v0.0.2`)를 입력하면 그 태그로 GitHub Release를 만들어 zip을 첨부합니다.

### 구조

```
src/main.rs                 진입점, 중복 실행 방지, 패닉 로그, 폰트 고정
src/app.rs                  UI·트레이·설정·엔진 연결
src/engine/                 WinDivert 바인딩(windivert.rs), 캡처·셰이핑 스레드(worker.rs)
src/filter.rs               정책 → WinDivert 필터 문자열 (방향, LAN 제외)
src/shaper.rs               토큰 버킷, 바이트 제한 큐
src/stats.rs                GetIfTable2 기반 실시간 속도
src/tray.rs, src/icon.rs    트레이 메뉴, 코드로 그린 아이콘
src/autostart.rs            작업 스케줄러 등록 (XML)
src/config.rs, src/i18n.rs  설정 저장, 한/영 문자열
ui/main.slint               메인 창 (Fluent 스타일)
```

## 라이선스

downcop은 [MIT 라이선스](LICENSE)로 배포됩니다.

### 서드파티

- **WinDivert**: LGPLv3 / GPLv2 듀얼 라이선스. downcop은 `WinDivert.dll`을 실행 시점에 동적으로 불러오므로 사용자가 DLL을 교체할 수 있습니다. 배포 zip에 `WinDivert-LICENSE.txt`를 포함합니다.
- **Slint**: Royalty-free License로 사용합니다. Made with [Slint](https://slint.dev).
