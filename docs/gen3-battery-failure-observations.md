# Oura Ring Gen 3 — 배터리 고장 진단 기록 (2026-10-12 ~ 14)

> **English TL;DR** — After moving a Gen 3 (Heritage, fw 3.4.3) onto this project
> ([setup notes](gen3-direct-sync-setup.md)), syncs kept succeeding but no heart-rate,
> motion or sleep data ever arrived. The ring's own logs show why: it charges normally
> (4.1–4.3 V on the dock) but **shuts down within minutes of leaving the charger** — 28 min,
> then 7 min, then **~2 min lying idle on a desk after a full charge** — and reboots every
> time it is put back on the dock. The battery gauge kept reporting "100 %" until each
> shutdown. Diagnosis: a worn-out battery (hardware); no software can fix it. This page
> records the evidence, how to read it from `oura.db`, and the two measurement mistakes
> made along the way.

결론부터: **링 배터리 고장입니다.** 동기화·앱·설정 문제가 아니었고, 공식 앱으로 돌아가도
같습니다. 같은 증상("동기화는 되는데 데이터가 없다")을 만나면 이 문서의 방법으로 몇 분 안에
판정할 수 있습니다.

## 환경

| 항목 | 값 |
|---|---|
| 링 | Oura Ring Gen 3 Heritage, 펌웨어 3.4.3, 하드웨어 `BLB_04B` |
| 셋업 | 공식 앱에서 공장 초기화 → 자체 인증 키 → 주간 HR·SpO2 automatic ([셋업 기록](gen3-direct-sync-setup.md)) |
| 기록 수집 | Mac CLI(`oura sync`)와 iOS 앱 양쪽. 링은 읽어간 이벤트를 지우지 않아 둘 다 전체 기록을 받음 |

## 증상

- iOS 앱·Mac CLI 모두 동기화 `OK` (이벤트 수백~수천 개)
- 그런데 수면·심박·HRV·SpO2·움직임(`motion` 0x47)·활동량(`activity_information` 0x50)이 **0건**
  — 0x47/0x50 은 기능 설정과 무관한 상시 스트림이라 0건 자체가 이상 신호
- 착용 중에는 Mac·iPhone 어느 쪽도 링을 찾지 못함 ("Your ring wasn't found", 동기화 버튼 무한 회전)
- 충전독에 올리면 바로 찾아짐
- 앱이 보여주는 배터리: 100% ↔ 1~23% 를 오락가락

## 근거 — 링이 스스로 남긴 기록

판정에 쓴 것은 전부 링의 history event 입니다. 앱의 해석이 아니라 펌웨어가 기록한 값입니다.

| 신호 | 이벤트 | 의미 |
|---|---|---|
| 링 시계 정지 | `time_sync`(0x42) 두 기준점 사이에서 실제 경과 > 링 카운터 경과 | 그 사이 링 전원이 꺼져 있었음 (brownout 은 카운터를 유지한 채 멈춤) |
| 재부팅 | `ring_start`(0x41) `reason: 0` | 꺼졌던 링이 충전기 전원으로 다시 켜짐 |
| 실제 배터리 | `debug_data` `battery_level_changed` 의 `voltage_mv` | % 가 아니라 **전압**을 볼 것 |
| 배터리 표시 | `debug_event` ASCII `batt: N` | 펌웨어가 보고하는 %. 이 링에서는 신뢰 불가 |
| 착용·충전 여부 | `temp_event`(0x46) 7채널 | 착용 34~35 °C / 충전 중 발열 ~35 °C / 충전 완료 후 독 위 31~33 °C / 책상 ~30 °C 이하로 하강 |
| 마지막 동작 시각 | 1분 간격 `temp_event` 의 마지막 기록 | 꺼진 시각 추정 |

### 세 번의 꺼짐

| 회차 | 조건 | 충전독에서 뺀 뒤 꺼질 때까지 | 꺼져 있던 시간 | 재부팅 시 전압 / 표시 |
|---|---|---|---|---|
| 1 (10-12 밤) | 착용, 공장 초기화 직후 짧게 충전 | 약 28분 | 8.5 h | 3.478 V / 1 % |
| 2 (10-13 밤) | 착용, **하루 종일 충전 후** | 약 7분 | 약 15분 | 3.696 V / 22 % |
| 3 (10-14 아침) | **착용 안 함, 책상 위**, 90분 이상 추가 충전 후 4.20 V 확인 | **약 2분** | 약 27분 | 3.693 V / 21 % |

3회차 상세 (통제 실험 — 착용·센서·무선 부하 없음):

```
~07:18  독 위        battery_level_changed 100 %  4200 mV
07:22   독에서 분리 (사용자 보고 시각)
~07:24  마지막 temp_event → 이후 링 카운터 정지         ← 꺼짐
        기준점 03:52 → 07:51: 실제 239.4 분, 링 카운터 212.3 분 (27 분 손실)
07:51   독에 올림    ring_start reason 0 → battery_level_changed 21 % 3693 mV
```

정상 Gen 3 는 착용·측정 상태로 4~7일 갑니다. 아무것도 측정하지 않는 상태에서 2분은
배터리가 부하를 거의 감당하지 못한다는 뜻입니다(용량 소실·내부저항 증가). 회차가 갈수록
짧아진 것도 열화가 진행 중임을 시사합니다.

### 충전 쪽은 정상

- 독 위에서 시간당 기록: 4.03 → 4.10 → 4.14 → … 4.31 V, 충전 완료 후 휴지 4.15~4.20 V
- 충전 중 발열(09~14시 약 35 °C) 후 완료되면 31~33 °C 로 하강
- 즉 충전 회로·충전독·접점은 정상이고, **충전기 전원이 빠지는 순간 배터리가 버티지 못함**

### 그래서 보였던 증상들

| 증상 | 설명 |
|---|---|
| 동기화는 되는데 데이터 0 | 측정을 시작하기 전에(또는 도중에) 꺼짐. 동기화는 독 위에서 성공하므로 "정상"으로 보임 |
| 착용 중 링이 안 보임 | 이미 꺼져 있음 |
| 독에 올릴 때마다 재부팅 | 꺼진 링이 충전기 전원으로 켜짐 |
| 표시 100 % ↔ 1~23 % | 독 위에서는 충전 전압으로 100 %, 재부팅 직후엔 실제 전압 기준 |

## `oura.db` 로 직접 판정하는 법

```sql
-- 1) 재부팅과 실제 전압 (재부팅 직후 3.7 V 이하 = 꺼졌다 켜진 것)
SELECT ring_timestamp, name,
       json_extract(decoded_json,'$.battery_pct')  AS pct,
       json_extract(decoded_json,'$.voltage_mv')   AS mv
FROM events
WHERE name = 'ring_start' OR decoded_json LIKE '%battery_level_changed%'
ORDER BY ring_timestamp;

-- 2) 시계 기준점: 인접한 두 점의 (unix 차) 와 (ring_timestamp 차 / 10) 를 비교.
--    실제 경과가 카운터보다 크게 앞서면 그만큼 꺼져 있었던 것
SELECT ring_timestamp, json_extract(decoded_json,'$.unix_time') AS unix
FROM events WHERE name = 'time_sync' ORDER BY ring_timestamp;

-- 3) 마지막 동작 시각과 착용 여부 (온도 7채널, 1분 간격)
SELECT ring_timestamp, json_extract(decoded_json,'$.temps_c') FROM events
WHERE name = 'temp_event' ORDER BY ring_timestamp DESC LIMIT 20;

-- 4) 상시 스트림이 오는지 (0 이면 측정 자체가 없음)
SELECT name, count(*) FROM events
WHERE name IN ('motion_event','activity_information','ibi_and_amplitude_event',
               'green_ibi_quality_event','hrv_event','spo2_r_pi_event','sleep_phase_data')
GROUP BY name;
```

iPhone 앱의 DB 는 Mac 에서 바로 꺼낼 수 있습니다:

```bash
xcrun devicectl device copy from --device <UDID> \
  --domain-type appDataContainer --domain-identifier <BUNDLE_ID> \
  --source "Library/Application Support/oura.db" --destination ./oura.db
```

### 통제 실험 절차 (배터리 수명 측정)

1. 독에서 **90분 이상** 충전 (전압이 아니라 시간으로 판정 — 아래 함정 1)
2. `oura sync` 로 기준점(`time_sync`)을 남기고, 분리 시각을 기록
3. 링을 **착용하지 않고 책상 위에** 둠
4. 60분 뒤(또는 언제든) 독에 올리고 `oura sync`
5. 위 SQL 로: 마지막 `temp_event` 시각, 카운터 정지 길이, `ring_start` 유무와 전압

재부팅이 없고 `temp_event` 가 끊김 없이 이어지면 배터리는 그 시간 동안 버틴 것입니다.

## 측정 중 저지른 실수 (재발 방지)

### 함정 1 — 충전 중 전압으로 "완충" 판정

`battery_level_changed` 의 전압은 충전 전류가 흐르는 동안 단자 전압이라 높게 나옵니다.
`≥ 4.10 V` 를 완충 기준으로 썼더니 첫 측정(4.294 V)에서 바로 통과했지만, 1분 뒤 같은 링이
**3.821 V / 54 %** 였습니다. 완충 판정은 **충전 시간(90분+)** 으로 하고, 전압은 독 위에서
충전이 끝난 뒤의 휴지 값(4.15~4.20 V)만 참고합니다.

### 함정 2 — macOS BLE 스캔이 사라진 기기를 계속 "있음" 으로 보고

1분마다 `oura scan` 으로 링의 존재를 감시했는데, 링이 꺼진 뒤 25분 동안에도 매번
`-70 dBm` 으로 "있음" 이었습니다. 같은 순간 연결은 60초 동안 실패했고, 필터 없는 `rawscan`
에도 링이 없었습니다. 신호 세기가 수십 번 **소수점까지 동일**하면 실제 수신이 아니라
캐시된 값을 의심해야 합니다. 생존 판정은 스캔이 아니라 **연결 성공** 또는 **링 자체 기록**으로.

또한 이 링은 충전독 밖에서는 (꺼지기 전에도) 광고가 잘 잡히지 않아, 스캔 유무만으로는
"꺼짐" 과 "광고 안 함" 을 구별할 수 없었습니다. 최종 판정은 항상 독에 올린 뒤의 링 기록으로 했습니다.

## 앱 쪽 후속 과제

- `ring_alerts` ([`oura-summary::ring_alerts`](../crates/oura-summary/src/ring_alerts.rs)) 는 링 시계
  정지 판정에 기존 날짜 계산과 같은 허용치(30분 / 2 %)를 씁니다. 그래서 1회차(8.5 h)는
  잡지만 **2·3회차(15분, 27분)는 경고가 뜨지 않습니다.** 재부팅 직후 전압도 3.69 V 라
  방전 기준(≤ 5 % 또는 ≤ 3.55 V)에 걸리지 않습니다.
- 개선안: "독에 올리는 순간의 `ring_start` + 그 직전 카운터 정지(허용치 미만이라도) + 직전
  기록이 `batt: 100`" 패턴을 **배터리 열화 경고**로. 위 세 번의 실제 기록이 회귀 테스트 데이터입니다.
- 앱 표시 배터리(%)는 이 링에서 신뢰할 수 없으므로, 전압 기반 표시나 경고 문구 병기가 필요합니다.

## 결론

| 항목 | 상태 |
|---|---|
| 공장 초기화 + 자체 키 + Mac/iOS 동기화 + Apple Health 경로 | ✅ 정상 동작 확인 |
| 링 충전 | ✅ 정상 |
| 링 배터리 | ❌ 완충 후 무부하 약 2분 — 사용 불가 |
| 대응 | 링 교체(이 프로젝트는 Gen 3 / Ring 4 / Ring 5 지원) 또는 제조사 문의 |
