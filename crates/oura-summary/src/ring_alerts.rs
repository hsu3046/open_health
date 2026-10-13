//! Ring health alerts — things the ring itself went through that explain missing data
//! (it was powered off, it ran out of battery, its battery gauge lied). A sync that
//! succeeds can still bring back an empty night; without these the apps say "up to
//! date" and the user is left guessing.
//!
//! Every message is composed here, once, so the web dashboard and the iOS app show
//! the same words (`ring_alerts` in the summary JSON).

use serde_json::{json, Value};

use crate::ring_time::{PowerGap, RingClock};
use crate::{date_label, hm};

/// Only alerts that ended within this many days of the newest ring time are shown.
const ALERT_WINDOW_DAYS: f64 = 14.0;
/// A battery reading this soon after a `ring_start` describes the reboot itself.
const BOOT_READING_WINDOW_DS: i64 = 600; // 60 s of ring counter
/// The reading the ring logged this long before a reboot is "what it claimed".
const PRE_BOOT_READING_WINDOW_DS: i64 = 36_000; // 1 h of ring counter
/// At or below either threshold a reboot is a battery-depletion restart.
const DEPLETED_PCT: i64 = 5;
const DEPLETED_MV: i64 = 3_550;
/// A depletion restart after the ring claimed at least this much means the gauge
/// cannot be trusted (e.g. it reset to 100% after a factory reset).
const GAUGE_CLAIMED_PCT: i64 = 50;

/// One battery observation from the ring's own logs: the decoded
/// `battery_level_changed` record (percent + voltage) or the firmware's
/// `batt: N` debug text (percent only).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Reading {
    pct: Option<i64>,
    mv: Option<i64>,
}

fn battery_reading(name: &str, json: &str) -> Option<Reading> {
    match name {
        "debug_data" if json.contains("battery_pct") => {
            let v: Value = serde_json::from_str(json).ok()?;
            Some(Reading {
                pct: v["battery_pct"].as_i64(),
                mv: v["voltage_mv"].as_i64().filter(|mv| *mv > 0),
            })
        }
        "debug_event" if json.contains("batt:") => {
            let v: Value = serde_json::from_str(json).ok()?;
            let pct = v["ascii"]
                .as_str()?
                .strip_prefix("batt:")?
                .trim()
                .parse()
                .ok()?;
            Some(Reading {
                pct: Some(pct),
                mv: None,
            })
        }
        _ => None,
    }
}

/// A `ring_start` with what the ring logged around it.
#[derive(Clone, Copy, Debug)]
struct Boot {
    ds: i64,
    captured_unix: i64,
    /// Lowest percent / voltage logged within a minute after the reboot.
    after_pct: Option<i64>,
    after_mv: Option<i64>,
    /// Last percent the ring claimed in the hour before the reboot.
    before_pct: Option<i64>,
}

impl Boot {
    fn depleted(&self) -> bool {
        self.after_pct.is_some_and(|p| p <= DEPLETED_PCT)
            || self.after_mv.is_some_and(|mv| mv <= DEPLETED_MV)
    }
    fn gauge_unreliable(&self) -> bool {
        self.depleted() && self.before_pct.is_some_and(|p| p >= GAUGE_CLAIMED_PCT)
    }
}

/// Walk outward from each `ring_start` in capture order. Neighbours in the drain are
/// neighbours on the ring, and stopping as soon as `ds` leaves the window keeps a
/// different boot's counter (which restarts near zero) from being mistaken for this one.
fn boots(events: &[(i64, u8, String, i64)], name_of: impl Fn(u8) -> &'static str) -> Vec<Boot> {
    let mut out = Vec::new();
    for (i, (ds, tag, _, cu)) in events.iter().enumerate() {
        if name_of(*tag) != "ring_start" {
            continue;
        }
        let mut boot = Boot {
            ds: *ds,
            captured_unix: *cu,
            after_pct: None,
            after_mv: None,
            before_pct: None,
        };
        for (eds, etag, json, _) in &events[i + 1..] {
            if *eds < *ds || *eds > ds + BOOT_READING_WINDOW_DS {
                break;
            }
            if let Some(r) = battery_reading(name_of(*etag), json) {
                boot.after_pct = match (boot.after_pct, r.pct) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
                boot.after_mv = match (boot.after_mv, r.mv) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
            }
        }
        for (eds, etag, json, _) in events[..i].iter().rev() {
            if *eds >= *ds || *eds < ds - PRE_BOOT_READING_WINDOW_DS {
                break;
            }
            if let Some(Reading { pct: Some(p), .. }) = battery_reading(name_of(*etag), json) {
                boot.before_pct = Some(p);
                break;
            }
        }
        out.push(boot);
    }
    out
}

fn when(unix: f64, tz: f64) -> String {
    format!("{} {}", date_label(unix, tz), hm(unix, tz))
}

fn battery_words(boot: &Boot) -> String {
    match (boot.after_pct, boot.after_mv) {
        (Some(p), Some(mv)) => format!("{p}%, {:.2} V", mv as f64 / 1000.0),
        (Some(p), None) => format!("{p}%"),
        (None, Some(mv)) => format!("{:.2} V", mv as f64 / 1000.0),
        (None, None) => "an empty battery".to_string(),
    }
}

fn gauge_sentence(boot: &Boot) -> String {
    format!(
        " It reported {}% shortly before shutting down, so its battery reading can't be \
         trusted yet: leave it on the charger for a full charge (2–3 h).",
        boot.before_pct.unwrap_or_default()
    )
}

fn gap_alert(gap: &PowerGap, boot: Option<&Boot>, tz: f64) -> Value {
    let hours = gap.lost_s / 3600.0;
    let depleted = boot.filter(|b| b.depleted());
    let mut message = if gap.boot_ds.is_some() {
        format!(
            "Ring was off for {hours:.1} h ({} → {}). Nothing was recorded during that time.",
            when(gap.off_from_unix, tz),
            hm(gap.off_until_unix, tz)
        )
    } else {
        format!(
            "Ring was off for about {hours:.1} h somewhere between {} and {}. Nothing was \
             recorded during that time.",
            when(gap.off_from_unix, tz),
            when(gap.off_until_unix, tz)
        )
    };
    if let Some(b) = depleted {
        message.push_str(&format!(
            " Its battery ran out; it restarted at {} once charged.",
            battery_words(b)
        ));
        if b.gauge_unreliable() {
            message.push_str(&gauge_sentence(b));
        }
    }
    json!({
        "kind": "ring_off",
        "severity": "warning",
        "cause": if depleted.is_some() { "battery" } else { "unknown" },
        "start_unix": gap.off_from_unix.round() as i64,
        "end_unix": gap.off_until_unix.round() as i64,
        "off_h": (hours * 10.0).round() / 10.0,
        "located": gap.boot_ds.is_some(),
        "battery_after_pct": depleted.and_then(|b| b.after_pct),
        "voltage_after_v": depleted.and_then(|b| b.after_mv).map(|mv| mv as f64 / 1000.0),
        "battery_before_pct": depleted.and_then(|b| b.before_pct),
        "gauge_unreliable": depleted.is_some_and(|b| b.gauge_unreliable()),
        "message": message,
    })
}

/// Alerts newest first. `name_of` maps an event tag to its protocol name.
pub(crate) fn ring_alerts(
    events: &[(i64, u8, String, i64)],
    clock: &RingClock,
    tz: f64,
    name_of: impl Fn(u8) -> &'static str,
) -> Vec<Value> {
    let boots = boots(events, name_of);
    let newest = clock.latest_unix() as f64;
    let cutoff = newest - ALERT_WINDOW_DAYS * 86_400.0;
    let mut alerts: Vec<(f64, Value)> = Vec::new();
    let mut explained = Vec::new();

    for gap in clock.power_gaps() {
        if gap.off_until_unix < cutoff {
            continue;
        }
        let boot = gap.boot_ds.and_then(|ds| boots.iter().find(|b| b.ds == ds));
        if let Some(b) = boot {
            explained.push(b.ds);
        }
        alerts.push((gap.off_until_unix, gap_alert(&gap, boot, tz)));
    }

    // A depletion restart the clock could not bracket (no anchor on one side) still
    // tells the user the battery ran out.
    for b in boots
        .iter()
        .filter(|b| b.depleted() && !explained.contains(&b.ds))
    {
        let r = clock.resolve(b.ds, b.captured_unix);
        if r.source.is_dated() && r.unix < cutoff {
            continue;
        }
        let at = if r.source.is_dated() {
            format!(" at {}", when(r.unix, tz))
        } else {
            String::new()
        };
        let mut message = format!(
            "Ring restarted after its battery ran out{at} ({}). Data from before the \
             restart may be missing.",
            battery_words(b)
        );
        if b.gauge_unreliable() {
            message.push_str(&gauge_sentence(b));
        }
        alerts.push((
            if r.source.is_dated() { r.unix } else { newest },
            json!({
                "kind": "battery_depleted",
                "severity": "warning",
                "cause": "battery",
                "end_unix": r.source.is_dated().then(|| r.unix.round() as i64),
                "battery_after_pct": b.after_pct,
                "voltage_after_v": b.after_mv.map(|mv| mv as f64 / 1000.0),
                "battery_before_pct": b.before_pct,
                "gauge_unreliable": b.gauge_unreliable(),
                "message": message,
            }),
        ));
    }

    alerts.sort_by(|a, b| b.0.total_cmp(&a.0));
    alerts.into_iter().map(|(_, v)| v).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(ds: i64, tag: u8, json: &str, captured: i64) -> (i64, u8, String, i64) {
        (ds, tag, json.to_string(), captured)
    }
    fn names(tag: u8) -> &'static str {
        oura_protocol::events::event_name(tag)
    }

    const RING_START: &str = r#"{"api_version":"2.0.0","bootloader_version":"1.0.1","firmware_version":"3.4.3","reason":0}"#;
    const NIGHT_SYNC: i64 = 1_791_817_650; // 2026-10-12 15:07:30 UTC
    const MORNING_SYNC: i64 = 1_791_849_878; // 2026-10-13 00:04:38 UTC

    /// The real Gen 3 (fw 3.4.3) night of 2026-10-12 → 13, reduced to the events that
    /// matter: anchors from the evening sync, "batt: 100" logged right up to the
    /// brownout, the counter frozen for ~8.5 h, then a reboot at 1% / 3.478 V when the
    /// ring was put on the charger at 09:00 local.
    fn depleted_night() -> Vec<(i64, u8, String, i64)> {
        vec![
            ev(2261, 0x42, r#"{"unix_time":1791816717}"#, NIGHT_SYNC),
            ev(10959, 0x42, r#"{"unix_time":1791817638}"#, NIGHT_SYNC),
            ev(
                11013,
                0x42,
                r#"{"source":"phone","unix_time":1791817650}"#,
                NIGHT_SYNC,
            ),
            ev(11456, 0x43, r#"{"ascii":"batt: 100"}"#, MORNING_SYNC),
            ev(
                27631,
                0x46,
                r#"{"temps_c":[34.42,33.62,29.04,39.36,34.5,33.04,32.8]}"#,
                MORNING_SYNC,
            ),
            ev(27655, 0x43, r#"{"ascii":"batt: 100"}"#, MORNING_SYNC),
            ev(27673, 0x41, RING_START, MORNING_SYNC),
            ev(
                27703,
                0x61,
                r#"{"battery_pct":1,"flag_a":0,"flag_b":1,"kind":"battery_level_changed","subtype":36,"voltage_mv":3478}"#,
                MORNING_SYNC,
            ),
            ev(27705, 0x43, r#"{"ascii":"batt: 1"}"#, MORNING_SYNC),
            ev(27718, 0x42, r#"{"unix_time":1791849941}"#, MORNING_SYNC),
            ev(
                28004,
                0x42,
                r#"{"source":"phone","unix_time":1791849870}"#,
                MORNING_SYNC,
            ),
            ev(
                28079,
                0x42,
                r#"{"source":"phone","unix_time":1791849878}"#,
                MORNING_SYNC,
            ),
        ]
    }

    #[test]
    fn real_gen3_brownout_night_is_one_located_battery_gap() {
        let events = depleted_night();
        let clock = RingClock::from_events(&events);
        let gaps = clock.power_gaps();
        assert_eq!(gaps.len(), 1, "{gaps:?}");
        let g = gaps[0];
        assert_eq!(g.boot_ds, Some(27673));
        // off at 15:35:16 UTC (00:35 KST), back at 00:05:37 UTC (09:05 KST)
        assert!(
            (g.off_from_unix - 1_791_819_316.0).abs() < 1.0,
            "{}",
            g.off_from_unix
        );
        assert!(
            (g.off_until_unix - 1_791_849_936.5).abs() < 1.0,
            "{}",
            g.off_until_unix
        );
        assert!((g.lost_s / 3600.0 - 8.51).abs() < 0.02, "{}", g.lost_s);

        let alerts = ring_alerts(&events, &clock, 9.0, names);
        assert_eq!(alerts.len(), 1, "{alerts:#?}");
        let a = &alerts[0];
        assert_eq!(a["kind"], "ring_off");
        assert_eq!(a["cause"], "battery");
        assert_eq!(a["located"], true);
        assert_eq!(a["off_h"], 8.5);
        assert_eq!(a["battery_after_pct"], 1);
        assert_eq!(a["voltage_after_v"], 3.478);
        assert_eq!(a["battery_before_pct"], 100);
        assert_eq!(a["gauge_unreliable"], true);
        let msg = a["message"].as_str().unwrap();
        assert!(
            msg.starts_with("Ring was off for 8.5 h (Tue 10-13 00:35 → 09:05)"),
            "{msg}"
        );
        assert!(msg.contains("restarted at 1%, 3.48 V"), "{msg}");
        assert!(
            msg.contains("It reported 100% shortly before shutting down"),
            "{msg}"
        );
    }

    #[test]
    fn agreeing_anchors_raise_nothing() {
        // Same evening, but the counter kept pace with the wall clock all night.
        let events = vec![
            ev(10959, 0x42, r#"{"unix_time":1791817638}"#, NIGHT_SYNC),
            ev(
                11013,
                0x42,
                r#"{"source":"phone","unix_time":1791817650}"#,
                NIGHT_SYNC,
            ),
            ev(11456, 0x43, r#"{"ascii":"batt: 100"}"#, MORNING_SYNC),
            ev(333_000, 0x43, r#"{"ascii":"batt: 62"}"#, MORNING_SYNC),
            ev(
                333_920,
                0x42,
                r#"{"source":"phone","unix_time":1791849878}"#,
                MORNING_SYNC,
            ),
        ];
        let clock = RingClock::from_events(&events);
        assert!(clock.power_gaps().is_empty());
        assert!(ring_alerts(&events, &clock, 9.0, names).is_empty());
    }

    #[test]
    fn stall_without_a_logged_reboot_is_reported_unlocated_without_a_cause() {
        let events = vec![
            ev(
                11013,
                0x42,
                r#"{"source":"phone","unix_time":1791817650}"#,
                NIGHT_SYNC,
            ),
            ev(27718, 0x42, r#"{"unix_time":1791849941}"#, MORNING_SYNC),
        ];
        let clock = RingClock::from_events(&events);
        let alerts = ring_alerts(&events, &clock, 9.0, names);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0]["located"], false);
        assert_eq!(alerts[0]["cause"], "unknown");
        assert_eq!(alerts[0]["gauge_unreliable"], false);
        let msg = alerts[0]["message"].as_str().unwrap();
        assert!(
            msg.contains("somewhere between Tue 10-13 00:07 and Tue 10-13 09:05"),
            "{msg}"
        );
    }

    #[test]
    fn healthy_reboot_is_not_a_battery_alert() {
        // A firmware reboot at 64% with no lost wall time: nothing to tell the user.
        let events = vec![
            ev(
                11013,
                0x42,
                r#"{"source":"phone","unix_time":1791817650}"#,
                NIGHT_SYNC,
            ),
            ev(20_000, 0x41, RING_START, MORNING_SYNC),
            ev(20_010, 0x43, r#"{"ascii":"batt: 64"}"#, MORNING_SYNC),
            ev(
                20_020,
                0x61,
                r#"{"battery_pct":64,"kind":"battery_level_changed","subtype":36,"voltage_mv":3910}"#,
                MORNING_SYNC,
            ),
            ev(
                21_000,
                0x42,
                r#"{"source":"phone","unix_time":1791818649}"#,
                MORNING_SYNC,
            ),
        ];
        let clock = RingClock::from_events(&events);
        assert!(ring_alerts(&events, &clock, 9.0, names).is_empty());
    }

    #[test]
    fn depleted_reboot_outside_any_bracket_still_alerts() {
        // The ring was flat long enough that only its post-reboot records survived:
        // no anchor before the reboot, so there is no gap to bracket it.
        let events = vec![
            ev(500, 0x41, RING_START, MORNING_SYNC),
            ev(
                530,
                0x61,
                r#"{"battery_pct":0,"kind":"battery_level_changed","subtype":36,"voltage_mv":3401}"#,
                MORNING_SYNC,
            ),
            ev(
                900,
                0x42,
                r#"{"source":"phone","unix_time":1791849878}"#,
                MORNING_SYNC,
            ),
        ];
        let clock = RingClock::from_events(&events);
        let alerts = ring_alerts(&events, &clock, 9.0, names);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0]["kind"], "battery_depleted");
        assert_eq!(alerts[0]["gauge_unreliable"], false);
        let msg = alerts[0]["message"].as_str().unwrap();
        assert!(
            msg.starts_with(
                "Ring restarted after its battery ran out at Tue 10-13 09:03 (0%, 3.40 V)"
            ),
            "{msg}"
        );
    }
}
