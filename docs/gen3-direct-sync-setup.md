# Oura Ring Gen 3 — 공식 앱 없이 직접 동기화하기 (실전 셋업 기록)

> **English TL;DR** — Field notes from moving an Oura Ring Gen 3 (Heritage, firmware
> 3.4.3) off the official app onto this project, on macOS + iPhone. Every failure we hit
> and its root cause is listed in the [troubleshooting table](#문제-해결-표). Key
> lessons: wrap BLE CLIs in an `.app` bundle on macOS (TCC kills bare binaries), a
> factory reset from the official app works, and after any reset **every** previously
> bonded device (Mac *and* iPhone) must "Forget This Device" or CoreBluetooth fails with
> error 14 *Peer removed pairing information*.

공식 Oura 앱이 구형 링의 동기화를 제대로 지원하지 않아, 링을 이 프로젝트(open_oura 기반)로
옮기면서 겪은 실패와 해결 방법을 순서대로 정리한 문서입니다. 같은 길을 가는 사람이 같은
함정을 다시 밟지 않도록, **증상 → 원인 → 해결**을 모두 남깁니다.

## 검증 환경

| 항목 | 값 |
|---|---|
| 링 | Oura Ring Gen 3 **Heritage** (내부 구조는 Horizon 과 동일한 Gen 3) |
| 펌웨어 / API / BT 스택 | 3.4.3 / 2.0.0 / 5.0.12 |
| 하드웨어 ID | `BLB_04B` |
| Mac | Apple Silicon, macOS 27.0, Xcode 27.0, Rust 1.93 |
| iPhone | iPhone 16 Pro Max (iOS 27.0.1) |
| 날짜 | 2026-10-12 ~ 13 |

## 한눈에 보는 전체 흐름

```
1. Mac 에서 CLI 빌드 → .app 번들로 감싸 Bluetooth 권한 획득
2. scan / info 로 링 감지·펌웨어 확인 (키 없이, 읽기 전용)
3. 공식 앱에서 마지막 동기화 → 공식 앱에서 "공장 초기화"
4. Mac·iPhone 양쪽에서 옛 Bluetooth 페어링 삭제 ("이 기기 지우기")
5. Mac 에서 pair → 자체 인증 키 설치 (키 파일 백업!)
6. features 로 심박·SpO2 측정 켜기 → sync
7. iOS 앱 빌드·설치 → 같은 키 입력 → Connect & sync → Apple Health 내보내기 켜기
```

---

## 0. 왜 공장 초기화가 필요한가 — 인증 키

링의 배터리·이력 이벤트·실시간 심박 같은 기능은 **16바이트 앱 인증 키**가 있어야 읽힙니다.
이 키는 공식 앱이 링을 처음 등록할 때 만들어 링과 폰에만 저장합니다.

| 키 확보 방법 | 링 데이터 | 조건 |
|---|---|---|
| 루팅된 Android 에서 공식 앱 DB 의 키 추출 | 보존 | 루팅 기기 필요 (`tools/android_oura_key_extract.py`, open_oura) |
| **공장 초기화 후 자체 키 설치** | **링 내 미동기화 기록 소실** | 링만 있으면 됨 |
| iPhone 에서 추출 | — | **불가능** (탈옥 없이는 앱 샌드박스 접근 불가) |

iPhone 사용자는 사실상 **공장 초기화 + 자체 키** 경로뿐입니다.

> ⚠️ 초기화 후 공식 앱에서 **"새 링 설정"을 하면 Oura 키가 다시 설치되어 이 프로젝트가
> 잠깁니다.** 되돌리려면 다시 공장 초기화해야 합니다. 공식 앱과 이 앱은 **둘 중 하나**입니다.

---

## 1. macOS 에서 CLI 로 링 감지하기

```bash
cargo build --release -p oura-cli        # → target/release/oura
```

### 함정 1 — 출력 없이 즉시 종료 (exit 134)

| | |
|---|---|
| 증상 | `oura scan` 이 **아무것도 출력하지 않고** 종료. 종료 코드 134(SIGABRT). Terminal.app 에서 돌려도 동일 |
| 진단 | `~/Library/Logs/DiagnosticReports/oura-*.ips` 의 `termination.namespace` 가 **`TCC`**, 사유 "`NSBluetoothAlwaysUsageDescription` 없음" |
| 원인 | macOS 는 Bluetooth 권한을 실행 파일이 아니라 **책임 앱(responsible process)** 의 Info.plist 로 판정. 맨 바이너리를 IDE·에이전트·터미널에서 띄우면 부모 앱에 사용 설명이 없을 때 OS 가 프로세스를 즉시 죽임 |
| 해결 | 바이너리를 **Bluetooth 사용 설명이 든 `.app` 번들로 감싸 `open` 으로 실행**. 그러면 번들 자체가 책임 앱이 되어 정상적으로 권한 팝업이 뜸 |

```bash
A=OuraProbe.app
mkdir -p "$A/Contents/MacOS"
cp target/release/oura "$A/Contents/MacOS/oura"
cat > "$A/Contents/Info.plist" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>com.example.ouraprobe</string>
  <key>CFBundleName</key><string>OuraProbe</string>
  <key>CFBundleExecutable</key><string>oura</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSBackgroundOnly</key><true/>
  <key>NSBluetoothAlwaysUsageDescription</key><string>Reads data directly from your Oura ring.</string>
</dict></plist>
EOF
codesign --force -s - "$A"          # ad-hoc 서명 (재서명할 때마다 권한을 다시 물을 수 있음)

# stdout/stderr 는 파일로 받는다 (open 은 터미널에 붙지 않음)
open -W -n --stdout scan.out --stderr scan.err "$A" --args --name "" --scan-timeout 20 scan
cat scan.out scan.err
```

### 함정 2 — 권한 허용 직후 첫 스캔이 "No Oura rings found."

권한 팝업이 떠 있는 동안 시작된 스캔은 Bluetooth 가 준비되기 전이라 아무것도 못 봅니다.
**허용한 뒤 한 번 더 실행**하면 됩니다. 링이 정말 없는지 의심되면 필터 없는 진단 스캔으로
주변 광고 기기 전체를 확인합니다 (open_oura 의 `rawscan` 예제):

```bash
# open_oura 저장소에서
cargo build --release -p oura-link --features ble --example rawscan
# 같은 .app 번들 방식으로 실행 → "*** OURA *** Oura Ring Gen3" 행이 보이면 링은 광고 중
```

Gen 3 는 광고 패킷에 Oura 서비스 UUID `98ed0001-a541-11e4-b6a0-0002a5d5c51b` 를 **싣습니다**
(실측). `scan` 이 못 찾는다면 필터 문제가 아니라 타이밍·권한·링크 점유 문제입니다.

### 함정 3 — `info` 가 "timed out connecting to the ring"

| | |
|---|---|
| 원인 | 링은 **BLE 링크를 한 번에 하나만** 허용. 공식 Oura 앱이 깔린 iPhone 이 링크를 쥐고 있음 |
| 해결 | iPhone **설정 앱 → Bluetooth 끄기** (제어 센터 토글은 BLE 를 완전히 끄지 않음) + 공식 Oura 앱 강제 종료 + 링을 충전독에 올려 Mac 옆에 두고 한 번 뺐다 다시 올리기 |

성공하면 키 없이도 이렇게 나옵니다:

```
Firmware : 3.4.3
API      : 2.0.0
BT stack : 5.0.12
Serial   : <SERIAL>
Hardware : BLB_04B
Caps     : 0:5 1:2 2:4 3:3 4:1 5:1 7:0 8:0 9:0 10:6 11:0 12:0 13:1
Battery  : <pass --key-file to read (auth required)>
```

---

## 2. 공장 초기화

### 방법 A — 공식 Oura 앱 (실제로 사용, 성공)

명령으로 링을 초기화하려면 **링과 페어링·인증된 기기**가 필요한데, 공식 앱이 바로 그 기기입니다.
가장 간단하고 확실한 방법입니다.

1. iPhone Bluetooth 를 켜고 공식 앱 실행 → **가능하면 먼저 동기화** (링에 남은 기록을 Oura 클라우드로 올려 보존)
2. 링 설정에서 **"공장 초기화(Factory reset)"** 실행
   - ⚠️ **"링 제거(계정에서 삭제)"와 다릅니다.** 계정에서만 빼면 링 안의 Oura 키가 남아, 이후 `pair` 가 거부됩니다
3. 공식 앱 강제 종료

### 방법 B — 충전독 뒤집기 (공식 앱이 링에 연결조차 못 할 때)

버튼 없는 Gen3/Ring 4 충전독용 하드웨어 절차입니다 (open_oura `docs/factory-reset.md`,
Ring 4 에서 검증. 이 문서 작성 시 Gen 3 에서는 방법 A 를 썼으므로 직접 검증하지 않음).

1. 링을 독에서 뺐다가 다시 올리고 **약 2초만** 대기
2. 독을 링째 **뒤집기 → 파란색**
3. **바로 세우기 → 빨간색**
4. **뒤집기 → 자홍/보라색**
5. **바로 세우기 → 노란색** (초기화 시작)
6. 몇 분 뒤 **파란색 깜빡임** = 완료

### 초기화로 지워지는 것

- 설치된 16바이트 인증 키, **모든 BLE 페어링(bond)**
- 링 내부의 이벤트 버퍼 (미동기화 기록)
- 사용자 신체 정보 (펌웨어 기본값으로 복귀)

### 초기화 확인 신호

광고 이름이 **`Oura <SERIAL>`** 로 바뀝니다 (키가 설치되면 다시 `Oura Ring Gen3`). macOS
CoreBluetooth 는 옛 이름을 캐시해 `Oura Ring Gen3 [Oura <SERIAL>]` 처럼 둘 다 보일 수 있습니다.

---

## 3. 자체 키 설치 (`pair`)

### 함정 4 — `pair` 가 "Peer removed pairing information"

| | |
|---|---|
| 증상 | `Error: connecting to ring … ble error: Runtime Error: Peer removed pairing information` (키 파일은 생성되지 않음, 링 상태 변화 없음 — 안전) |
| 원인 | 초기화 **전**에 Mac 이 `info` 로 연결하면서 링과 페어링 정보를 저장함. 초기화가 링 쪽 페어링을 지웠으므로 Mac 의 옛 정보로는 접속이 거절됨 (CoreBluetooth `CBError.peerRemovedPairingInformation`, code 14) |
| 해결 | Mac **시스템 설정 → Bluetooth** → 링 항목 ⓘ → **"이 기기 지우기"** (목록에 없으면 Bluetooth 끄고 켜기) → 재시도. 이번엔 **페어링 요청 팝업**이 뜨므로 수락 |

> Gen 3 는 초기화 후 **BLE 링크 암호화(페어링)가 필수**입니다. 팝업을 수락하기 전에는 어떤
> 명령도 써지지 않습니다 (`Encryption is insufficient`).

### 키 설치

```bash
mkdir -p keys && chmod 700 keys && printf '*\n' > keys/.gitignore
open -W -n --stdout pair.out --stderr pair.err OuraProbe.app \
  --args --name "" --key-file "$PWD/keys/oura-<SERIAL>.key" pair
cat pair.out pair.err
```

```
Installed new auth key on <SERIAL>; saved to …/keys/oura-<SERIAL>.key
Authenticated: Success
Battery: 100%
```

- `--name ""` 필수: 초기화된 링은 `Oura <SERIAL>` 로 광고하므로 기본 필터와 다를 수 있음
- 키 파일은 **설치 전에 먼저 저장**됩니다 (도중 크래시로 유일한 키를 잃지 않도록)
- 🔑 **키 파일을 반드시 별도 백업.** 링에서 다시 읽어낼 방법이 없어, 잃으면 공장 초기화뿐
- 키 파일은 `0600`, git 에 절대 커밋하지 않기 (이 저장소 `.gitignore` 는 `*.key`, `*.hex` 를 제외)

---

## 4. 측정 기능 켜기 + 첫 동기화

자체 키로 페어링한 직후에는 측정 기능이 **꺼져 있습니다** (공식 앱이 온보딩 때 켜던 것).

```bash
oura --name "" --key-file keys/oura-<SERIAL>.key features --enable-hr --enable-spo2
#   daytime HR: mode=automatic …
#         SpO2: mode=automatic …
#   resting HR: mode=automatic …

oura --name "" --key-file keys/oura-<SERIAL>.key --db data/oura.db sync
oura --db data/oura.db events
```

(Mac 에서는 위 명령도 모두 `.app` 번들 + `open --args` 방식으로 실행)

관찰 결과:

- 초기화 직후 첫 sync: 이벤트 99개 (`debug_*`, `ble_connection`, `time_sync`, `temp_event`, `ring_start`)
- **링은 읽어간 이벤트를 지우지 않습니다.** Mac 과 iPhone 이 각각 커서 0 부터 받아도 둘 다
  전부 받았습니다 → **여러 기기가 독립적으로 동기화 가능** (커서는 클라이언트 쪽에 저장)
- 측정은 링이 스스로 시점을 정하므로, 착용 후 몇 분~수 시간이 지나야 HR/HRV/SpO2 가 쌓입니다.
  수면 단계(`sleep_phase_data` 0x5a)는 하룻밤 착용 후 확인

---

## 5. iOS 앱 빌드·설치

모델 없는 빌드 구성(`project-ci.yml`)을 사용합니다. Oura 의 독점 ML 모델(.ptl)은 포함되지
않으며, BLE 동기화·요약·Apple Health 내보내기는 모델 없이 동작합니다.

```bash
./apps/ios/build-xcframework.sh                     # Rust 코어 (기기 + 시뮬레이터)
cd apps/ios/OuraApp && xcodegen generate --spec project-ci.yml
```

### 함정 5 — 시뮬레이터 빌드가 링크 단계에서 실패

| | |
|---|---|
| 증상 | `ld: warning: ignoring file …liboura_core.a: found architecture 'arm64', required architecture 'x86_64'` → BUILD FAILED |
| 원인 | `generic/platform=iOS Simulator` 는 x86_64 슬라이스까지 빌드하는데, Rust xcframework 의 시뮬레이터 슬라이스는 **arm64 전용** |
| 해결 | `ARCHS=arm64 ONLY_ACTIVE_ARCH=YES` 지정 |

```bash
xcodebuild -project OuraApp.xcodeproj -scheme OuraApp -sdk iphonesimulator \
  -destination 'generic/platform=iOS Simulator' -derivedDataPath ../DerivedData \
  ARCHS=arm64 ONLY_ACTIVE_ARCH=YES CODE_SIGNING_ALLOWED=NO build
```

### 함정 6 — 기기 빌드가 "No Accounts" + HealthKit 프로파일 오류

| | |
|---|---|
| 증상 | `error: No Accounts` + `Provisioning profile "iOS Team Provisioning Profile: *" doesn't include the HealthKit capability` |
| 원인 | Xcode 에 Apple 계정이 있어도, 에이전트·스크립트 등 다른 앱이 띄운 `xcodebuild` 는 그 계정의 로그인 토큰(키체인)에 접근하지 못함 → HealthKit 이 켜진 App ID/프로파일을 자동 생성 못하고 와일드카드 프로파일로 떨어짐 |
| 해결 | **최초 1회는 Xcode 앱(GUI)** 에서 기기 선택 → Signing & Capabilities 에서 Team 확인 → ▶ Run. Xcode 가 App ID + HealthKit 프로파일을 생성. 이후엔 명령줄 빌드 가능 |

### 함정 7 — Xcode 빌드는 성공했는데 iPhone 에 설치되지 않음

빌드 산출물(서명·HealthKit 권한·기기 등록)이 정상이면 `devicectl` 로 직접 설치·실행합니다:

```bash
APP=~/Library/Developer/Xcode/DerivedData/OuraApp-*/Build/Products/Debug-iphoneos/OuraApp.app
codesign -dv "$APP"                                         # Identifier / TeamIdentifier 확인
xcrun devicectl list devices                                # 기기 UDID 확인
xcrun devicectl device install app --device <UDID> "$APP"
xcrun devicectl device process launch --device <UDID> <BUNDLE_ID>
```

처음 실행 시 iPhone 에서 **설정 → 일반 → VPN 및 기기 관리 → 개발자 신뢰**, 필요하면
**설정 → 개인정보 보호 및 보안 → 개발자 모드** 켜기.

---

## 6. iPhone 앱에서 연결

1. iPhone Bluetooth 켜기
2. 앱의 키 입력칸에 **Mac 에서 만든 같은 키(32자 hex)** 붙여넣기
   - 채팅·메신저로 보내지 말고, 같은 Apple ID 의 공유 클립보드를 쓰면 편함: `tr -d ' \n' < keys/oura-<SERIAL>.key | pbcopy`
3. **Connect & sync**
4. ⚠️ **"Pair a factory-reset ring" 은 누르지 않기** — 새 키가 만들어져 Mac 의 키가 무용지물이 됨
5. 프로필에서 **Apple Health 내보내기** 켜기 → 권한 팝업 허용

### 함정 8 — 앱이 "Connection lost. Retrying (n of 6)…" 만 반복

| | |
|---|---|
| 증상 | 링은 찾는데(신호 정상) 연결 6회 모두 즉시 실패, 화면엔 "Connection lost" 뿐 |
| 진단 | 앱 진단 로그에 `[ble] GATT connect FAILED: … CBErrorDomain Code=14 "Peer removed pairing information"` |
| 원인 | 함정 4 와 동일 — **iPhone 에도 공식 앱 시절의 옛 페어링 정보**가 남아 있음 |
| 해결 | iPhone **설정 → Bluetooth** → 링 ⓘ → **"이 기기 지우기"** → 앱에서 다시 Connect & sync → 페어링 요청 수락 |

앱 진단 로그는 Mac 에서 USB/Wi-Fi 로 바로 꺼내 볼 수 있습니다:

```bash
xcrun devicectl device copy from --device <UDID> \
  --domain-type appDataContainer --domain-identifier <BUNDLE_ID> \
  --source "Library/Application Support/diagnostics/session.log" \
  --destination ./session.log
grep -E "\[(sync|ble)\]" session.log | tail -40
```

성공 시 로그:

```
[ble]  GATT connected (maxWrite=512B); discovering the Oura service
[ble]  all notify subscriptions confirmed; BLE link ready, handing to Rust auth
[sync] OK run=… inserted=267 events=267 cursor=…
[health] export 0 samples, 0 workouts      ← 수면 기록이 쌓이기 전이라 0 이 정상
```

---

## 문제 해결 표

| # | 증상 | 원인 | 해결 |
|---|---|---|---|
| 1 | CLI 가 출력 없이 종료, exit 134 | macOS TCC: 책임 앱에 `NSBluetoothAlwaysUsageDescription` 없음 | `.app` 번들로 감싸 `open` 으로 실행 |
| 2 | 권한 허용 직후 스캔 0건 | 팝업 대기 중 Bluetooth 미준비 | 허용 후 재실행 / `rawscan` 으로 확인 |
| 3 | `info` 연결 타임아웃 | iPhone 공식 앱이 단일 BLE 링크 점유 | iPhone 설정에서 Bluetooth 끄기 + 공식 앱 종료 + 링 충전독 |
| 4 | `pair`: Peer removed pairing information | 초기화로 링 쪽 페어링 삭제, Mac 의 옛 정보로 접속 | macOS Bluetooth 에서 "이 기기 지우기" → 페어링 팝업 수락 |
| 5 | 시뮬레이터 링크 실패 (x86_64) | Rust 시뮬레이터 슬라이스 arm64 전용 | `ARCHS=arm64 ONLY_ACTIVE_ARCH=YES` |
| 6 | No Accounts / HealthKit 프로파일 없음 | 외부에서 띄운 xcodebuild 는 Xcode 계정 토큰 접근 불가 | 최초 1회 Xcode GUI 로 Run |
| 7 | 빌드 성공, 설치 안 됨 | Xcode 설치 단계 중단 | `devicectl device install app` + `process launch` |
| 8 | iOS 앱 "Connection lost" 반복 | iPhone 의 옛 페어링 (CBError 14) | iOS Bluetooth 에서 "이 기기 지우기" |
| — | `pair` 가 키 설치 거부 | "링 제거"만 하고 공장 초기화는 안 됨 (Oura 키 잔존) | 공식 앱 Factory reset 또는 충전독 절차 |
| — | 이 프로젝트가 다시 잠김 | 공식 앱에서 "새 링 설정" 을 함 | 다시 공장 초기화 후 **같은 키 파일로** `pair` (이력 귀속 유지) |

## 체크리스트

- [ ] 초기화 **전** 공식 앱에서 마지막 동기화 (남은 기록 보존)
- [ ] 공식 앱의 **공장 초기화** (링 제거 아님)
- [ ] Mac·iPhone **양쪽** Bluetooth 에서 링 "이 기기 지우기"
- [ ] `pair` 후 키 파일 **백업** (0600, git 제외)
- [ ] `features --enable-hr --enable-spo2`
- [ ] iOS 앱엔 **같은 키** 입력, "Pair a factory-reset ring" 금지
- [ ] Apple Health 내보내기 켜기 (Mac 은 HealthKit 데이터 기록 불가 — iPhone 담당)
- [ ] 공식 앱으로 다시 링 등록하지 않기

## 참고

- 프로토콜·초기화·키 상세: [open_oura `docs/`](https://github.com/Th0rgal/open_oura/tree/main/docs)
  (`factory-reset.md`, `horizon-ring3-protocol-cheatsheet.md`, `ring-features.md`)
- iOS 빌드 일반: [`apps/ios/TESTFLIGHT.md`](../apps/ios/TESTFLIGHT.md)
