//! Tests for everything that doesn't need a screen, a camera or speakers.
//! `cargo test` runs them, and GitHub Actions runs them on Windows, macOS
//! and Linux for every push.

use std::time::{Duration, Instant};

use crate::audio::{AudioCmd, BellPattern, Pattern};
use crate::engine::panel::{KpMode, Out, Panel, PanelSave};
use crate::engine::types::*;

fn setup() {
    std::env::set_var("ROOMGUARD_DATA_DIR", std::env::temp_dir().join("roomguard-tests"));
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn panel(model: Model) -> Panel {
    setup();
    let labels = crate::config::default_labels(model);
    let mut p = Panel::new(model, PanelSave::fresh(model), labels, "Tester".to_string(), Instant::now());
    p.out.clear();
    p
}

fn keys(p: &mut Panel, ks: &str, t: Instant) {
    for c in ks.chars() {
        let k = match c {
            '*' => Key::Star,
            '#' => Key::Hash,
            d => Key::Digit(d.to_digit(10).expect("a digit") as u8),
        };
        p.key(k, true, t);
        p.key(k, false, t);
    }
}

/// Arms a PowerSeries panel away: code, then out through the door.
fn armed_away(p: &mut Panel, t0: Instant) {
    keys(p, "1234", t0);
    assert!(p.exit.is_some(), "the code starts the exit delay");
    p.set_zone(1, true, false, false, t0 + secs(5));
    p.set_zone(1, false, false, false, t0 + secs(7));
    p.tick(t0 + secs(13), 0);
    assert!(matches!(p.armed, Some(a) if a.mode == ArmMode::Away), "armed away once the door closed");
}

#[test]
fn powerseries_arms_away_and_disarms() {
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1832);
    armed_away(&mut p, t0);
    keys(&mut p, "1234", t0 + secs(20));
    assert!(p.armed.is_none());
}

#[test]
fn entry_delay_runs_out_into_an_alarm() {
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1832);
    armed_away(&mut p, t0);
    p.set_zone(1, true, false, false, t0 + secs(20));
    assert!(p.entry.is_some(), "the door starts the entry delay");
    p.tick(t0 + secs(51), 0);
    assert!(p.alarms.iter().any(|a| a.kind == AlarmKind::Burglary && a.zone == 1));
    assert_eq!(p.bell_sent, BellPattern::Steady);
}

#[test]
fn code_during_entry_delay_disarms_without_alarm() {
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1832);
    armed_away(&mut p, t0);
    p.set_zone(1, true, false, false, t0 + secs(20));
    keys(&mut p, "1234", t0 + secs(25));
    p.tick(t0 + secs(60), 0);
    assert!(p.armed.is_none());
    assert!(p.alarms.is_empty());
}

#[test]
fn walking_in_without_the_door_is_an_instant_alarm() {
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1832);
    armed_away(&mut p, t0);
    p.set_zone(2, true, false, false, t0 + secs(20));
    assert!(p.alarms.iter().any(|a| a.zone == 2), "an interior zone with no entry delay running alarms at once");
}

#[test]
fn stay_key_bypasses_the_room_zone() {
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1832);
    p.key(Key::Stay, true, t0);
    p.tick(t0 + Duration::from_millis(2100), 0);
    p.key(Key::Stay, false, t0 + Duration::from_millis(2200));
    assert!(p.exit.is_some(), "holding Stay arms without a code");
    p.tick(t0 + secs(250), 0);
    assert!(matches!(p.armed, Some(a) if a.mode == ArmMode::Stay));
    p.set_zone(2, true, false, false, t0 + secs(260));
    p.tick(t0 + secs(261), 0);
    assert!(p.alarms.is_empty(), "zone 2 is bypassed in stay mode");
}

#[test]
fn pc1550_code_arming_beeps_six_times() {
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1550);
    keys(&mut p, "1234", t0);
    assert!(p.exit.is_some());
    assert!(p.out.iter().any(|o| matches!(o, Out::Audio(AudioCmd::Beeps { count: 6, .. }))));
}

#[test]
fn not_ready_refuses_to_arm() {
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1832);
    p.set_zone(3, true, false, false, t0);
    keys(&mut p, "1234", t0);
    assert!(p.exit.is_none());
    assert!(p.armed.is_none());
}

#[test]
fn five_wrong_codes_lock_the_keypad() {
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1832);
    for _ in 0..5 {
        keys(&mut p, "0000", t0);
    }
    assert!(p.lockout_until.is_some());
}

#[test]
fn installer_programming_changes_the_entry_delay() {
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1832);
    keys(&mut p, "*85555", t0);
    assert!(p.is_programming());
    keys(&mut p, "005", t0);
    keys(&mut p, "01", t0);
    keys(&mut p, "010045120", t0);
    keys(&mut p, "#", t0);
    assert!(!p.is_programming());
    assert_eq!(p.entry_secs(ZoneKind::Delay1), 10);
}

#[test]
fn pc1550_programming_keeps_finished_entries() {
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1550);
    keys(&mut p, "*81500", t0);
    assert!(p.is_programming());
    keys(&mut p, "17", t0);
    keys(&mut p, "015", t0);
    keys(&mut p, "#", t0);
    assert_eq!(p.entry_secs(ZoneKind::Delay1), 15);
    assert_eq!(p.exit_secs(), 120, "untouched entries keep their value");
}

#[test]
fn a_new_user_code_works() {
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1832);
    keys(&mut p, "*51234", t0);
    assert!(matches!(p.kp, KpMode::UserCodes { .. }));
    keys(&mut p, "01", t0);
    keys(&mut p, "4321", t0);
    keys(&mut p, "#", t0);
    keys(&mut p, "4321", t0);
    assert!(p.exit.is_some(), "the new code arms");
}

#[test]
fn holding_fire_sounds_a_temporal_fire_alarm() {
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1832);
    p.key(Key::Fire, true, t0);
    p.tick(t0 + Duration::from_millis(2050), 0);
    assert!(p.alarms.iter().any(|a| a.kind == AlarmKind::Fire));
    p.tick(t0 + Duration::from_millis(2100), 0);
    assert_eq!(p.bell_sent, BellPattern::Temporal3);
}

#[test]
fn a_short_tap_on_fire_does_nothing() {
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1832);
    p.key(Key::Fire, true, t0);
    p.key(Key::Fire, false, t0 + Duration::from_millis(300));
    p.tick(t0 + secs(3), 0);
    assert!(p.alarms.is_empty());
}

#[test]
fn unplugging_shows_an_ac_trouble() {
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1832);
    p.inputs.ac_ok = false;
    p.tick(t0 + secs(1), 0);
    assert!(p.trouble_list().iter().any(|t| t.contains("AC")));
}

#[test]
fn trouble_beeps_until_acknowledged() {
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1832);
    p.inputs.audio_ok = false;
    p.tick(t0 + secs(1), 0);
    assert!(matches!(p.sounder_sent, Pattern::Beeps { count: 2, .. }));
    keys(&mut p, "#", t0 + secs(2));
    p.tick(t0 + secs(3), 0);
    assert_eq!(p.sounder_sent, Pattern::Off);
}

#[test]
fn the_lcd_shows_ready_to_arm() {
    let t0 = Instant::now();
    let p = panel(Model::Pc1832);
    let lcd = p.lcd_1832(t0, 0);
    assert!(lcd.lines[0].starts_with("System is"));
    assert_eq!(lcd.lines[1].trim(), "Ready to Arm");
    assert!(lcd.lines.iter().all(|l| l.chars().count() == 16));
}

#[test]
fn pc1550_lights_follow_the_zones() {
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1550);
    assert!(p.leds_1550(t0)[0].lit(0.0), "Ready is on");
    p.set_zone(3, true, false, false, t0);
    let l = p.leds_1550(t0);
    assert!(!l[0].lit(0.0), "Ready goes out");
    assert!(l[5 + 2].lit(0.0), "zone 3's light comes on");
}

#[test]
fn panels_survive_a_save_and_reload() {
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1832);
    armed_away(&mut p, t0);
    let json = serde_json::to_string(&p.to_save(false)).expect("saves");
    let back: PanelSave = serde_json::from_str(&json).expect("loads");
    let p2 = Panel::new(Model::Pc1832, back, crate::config::default_labels(Model::Pc1832), "Tester".to_string(), Instant::now());
    assert!(p2.armed.is_some(), "comes back armed, like a panel after a power cut");
}

#[test]
fn contact_id_checksum_makes_a_multiple_of_15() {
    let r = Report {
        id: 1,
        kind: ReportKind::Alarm(AlarmKind::Burglary),
        cid: 130,
        qualifier: 'E',
        zone_or_user: 1,
        zone_label: String::new(),
        text: String::new(),
        delay_s: 0,
        ts: 0,
    };
    let msg = crate::engine::operator::contact_id_message(&r, "1234");
    assert_eq!(msg.len(), 16);
    assert!(msg.starts_with("1234181130"));
    let sum: u32 = msg
        .chars()
        .map(|c| match c.to_digit(16).expect("hex digit") {
            0 => 10,
            v => v,
        })
        .sum();
    assert_eq!(sum % 15, 0);
}

#[test]
fn yuyv_frames_decode_to_rgb() {
    let (w, h) = (8usize, 4usize);
    let mut buf = Vec::new();
    for _ in 0..(w * h / 2) {
        buf.extend_from_slice(&[128, 128, 128, 128]);
    }
    let img = crate::vision::frame::decode(&buf, w, h, crate::vision::frame::Hint::Yuyv, 640).expect("decodes");
    assert_eq!((img.w, img.h), (w, h));
    assert!(img.data[0..3].iter().all(|&v| (120..=140).contains(&v)), "{:?}", &img.data[0..3]);
}

#[test]
fn nv12_frames_decode_to_rgb() {
    let (w, h) = (8usize, 4usize);
    let buf = vec![128u8; w * h + w * h / 2];
    let img = crate::vision::frame::decode(&buf, w, h, crate::vision::frame::Hint::Nv12, 640).expect("decodes");
    assert_eq!(img.data.len(), w * h * 3);
}

#[test]
fn the_tracker_predicts_ahead_of_a_walking_person() {
    use crate::vision::track::{Blob, Tracker};
    let mut tr = Tracker::new();
    let t0 = Instant::now();
    for i in 0..15u64 {
        let x = 0.2 + i as f32 * 0.02;
        tr.update(&[Blob { cx: x, cy: 0.5, w: 0.1, h: 0.3, px: 200 }], t0 + Duration::from_millis(i * 66));
    }
    let t = tr.tracks.first().expect("a track");
    assert!(t.confirmed);
    assert!(t.vel().0 > 0.1, "moving right at {}", t.vel().0);
    assert!(t.predict(0.5).0 > t.pos().0 + 0.05, "predicts further right");
}

#[test]
fn the_room_map_learns_a_doorway_from_foot_traffic() {
    use crate::vision::map::RoomMap;
    use crate::vision::track::Finished;
    let mut m = RoomMap::default();
    for i in 0..5u32 {
        let path: Vec<(f32, f32)> = (0..10).map(|k| (0.2 + k as f32 * 0.05, 0.8)).collect();
        let f = Finished { id: i, first: (0.2, 0.8), last: (0.21, 0.8), last_vel: (0.0, 0.0), path, secs: 3.0, known: false };
        m.learn(&f);
    }
    let door = m.suggest_doorway().expect("a doorway suggestion");
    assert!(door.contains(0.2, 0.8));
}

#[test]
fn osc_messages_are_padded_and_big_endian() {
    let m = crate::vision::ptz::osc_message("/OBSBOT/WebCam/General/SetGimMotorDegree", &[50, 10, -5]);
    assert_eq!(m.len() % 4, 0);
    assert!(m.starts_with(b"/OBSBOT/"));
    assert_eq!(&m[m.len() - 4..], &(-5i32).to_be_bytes());
}

#[test]
fn the_pan_calibration_measures_a_shift() {
    use crate::vision::ptz::{best_shift, profile};
    let (w, h) = (160usize, 60usize);
    let col = |x: usize| ((x * 37 + (x * x) % 91) % 251) as u8;
    let a: Vec<u8> = (0..w * h).map(|i| col(i % w)).collect();
    let b: Vec<u8> = (0..w * h).map(|i| col((i % w + 12).min(w - 1))).collect();
    let (shift, score) = best_shift(&profile(&a, w, h), &profile(&b, w, h), 100).expect("a match");
    assert!((shift - 12.0).abs() < 1.5, "shift {}", shift);
    assert!(score > 0.5);
}

#[test]
fn the_face_cascade_loads() {
    let c = crate::vision::faces::Cascade::load(include_bytes!("../assets/cascade/lbp_frontalface.bin"));
    assert!(c.is_some());
}

#[test]
fn the_demo_camera_draws_frames() {
    let img = crate::vision::demo::scene(5.0);
    assert_eq!(img.data.len(), 640 * 480 * 3);
    assert!(crate::vision::demo::walker(5.0).is_some());
    assert!(crate::vision::demo::walker(1.0).is_none());
}

#[test]
fn every_siren_stays_finite_and_in_range() {
    for model in SirenModel::ALL {
        for warble in [false, true] {
            let mut s = crate::audio::synth::Synth::new(48_000.0);
            s.apply(AudioCmd::Siren { model, warble });
            s.apply(AudioCmd::Bell(BellPattern::Steady));
            s.apply(AudioCmd::Sounder(Pattern::Steady));
            let mut peak = 0.0f32;
            for _ in 0..48_000 {
                let v = s.next();
                assert!(v.is_finite(), "{:?} produced a bad sample", model);
                peak = peak.max(v.abs());
            }
            assert!(peak > 0.01, "{:?} was silent", model);
            assert!(peak <= 1.0);
        }
    }
}

#[test]
fn keypad_beeps_make_sound_then_stop() {
    let mut s = crate::audio::synth::Synth::new(48_000.0);
    s.apply(AudioCmd::Beeps { count: 2, on_ms: 70, off_ms: 70 });
    let loud: f32 = (0..4_800).map(|_| s.next().abs()).fold(0.0, f32::max);
    for _ in 0..48_000 {
        s.next();
    }
    let quiet: f32 = (0..4_800).map(|_| s.next().abs()).fold(0.0, f32::max);
    assert!(loud > 0.05);
    assert!(quiet < 0.01);
}

#[test]
fn helpers_encode_correctly() {
    assert_eq!(crate::util::base64(b"hello"), "aGVsbG8=");
    assert_eq!(crate::util::base64(b"hi"), "aGk=");
    let bytes = vec![0u8, 1, 127, 255];
    assert_eq!(crate::util::hex_decode(&crate::util::hex_encode(&bytes)), Some(bytes));
    assert_eq!(crate::util::slug("Logi C920 HD Pro!"), "logi-c920-hd-pro");
}

#[test]
fn version_comparison_is_numeric() {
    assert!(crate::update::newer("2.10.0", "2.9.9"));
    assert!(crate::update::newer("v2.0.1", "2.0.0"));
    assert!(!crate::update::newer("2.0.0", "2.0.0"));
    assert!(!crate::update::newer("1.9.0", "2.0.0"));
}

#[test]
fn settings_survive_a_round_trip_and_old_files_load() {
    let c = crate::config::AppConfig::default();
    let back: crate::config::AppConfig = serde_json::from_str(&serde_json::to_string(&c).expect("saves")).expect("loads");
    assert_eq!(c, back);
    let old: crate::config::AppConfig = serde_json::from_str(r#"{"admin_name":"Kaushal"}"#).expect("an old file loads");
    assert_eq!(old.admin_name, "Kaushal");
    assert!(old.close_needs_code);
}

#[test]
fn png_files_are_well_formed() {
    let px = vec![255u8; 3 * 2 * 4];
    let png = crate::png::encode(3, 2, &px);
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(png.ends_with(&[0xAE, 0x42, 0x60, 0x82]), "ends with the IEND checksum");
}
