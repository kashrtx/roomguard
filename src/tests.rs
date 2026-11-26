//! Tests for everything that doesn't need a screen, a camera or speakers.
//! `cargo test` runs them, and GitHub Actions runs them on Windows, macOS
//! and Linux for every push.

use std::time::{Duration, Instant};

use crate::audio::{AudioCmd, BellPattern, Pattern};
use crate::engine::panel::{ArmHow, KpMode, Out, Panel, PanelSave};
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

/// Arms a PowerSeries panel away: code, then out through the door. Exit
/// delay termination ([014] option 7, off from the factory) is turned on so
/// the exit time drops to five seconds once the door closes, which keeps
/// these tests short.
fn armed_away(p: &mut Panel, t0: Instant) {
    p.ee.sections.insert("014".to_string(), "67".to_string());
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
fn keypad_lockout_follows_section_012() {
    let t0 = Instant::now();
    // From the factory the lockout is off ([012] 000 000).
    let mut p = panel(Model::Pc1832);
    for _ in 0..6 {
        keys(&mut p, "0000", t0);
    }
    assert!(p.lockout_until.is_none(), "no lockout from the factory");
    // Programmed for 5 wrong codes and 5 minutes, it locks out.
    let mut p = panel(Model::Pc1832);
    p.ee.sections.insert("012".to_string(), "005005".to_string());
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
fn holding_fire_sounds_the_programmed_fire_pattern() {
    let t0 = Instant::now();
    // [013] option 8 is off from the factory: the standard pulsed fire bell.
    let mut p = panel(Model::Pc1832);
    p.key(Key::Fire, true, t0);
    p.tick(t0 + Duration::from_millis(2050), 0);
    assert!(p.alarms.iter().any(|a| a.kind == AlarmKind::Fire));
    p.tick(t0 + Duration::from_millis(2100), 0);
    assert_eq!(p.bell_sent, BellPattern::Pulsed);
    // With option 8 on, Temporal Three.
    let mut p = panel(Model::Pc1832);
    p.ee.sections.insert("013".to_string(), "35678".to_string());
    p.key(Key::Fire, true, t0);
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
        tr.update(&[Blob { cx: x, cy: 0.5, w: 0.1, h: 0.3, px: 200, strength: 0, motion: 1.0 }], t0 + Duration::from_millis(i * 66));
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

#[test]
fn temporal4_co_alarm_and_buzzer_pitch_work() {
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1832);
    // Program Zone 3 as [41] 24-Hr Carbon Monoxide via installer programming.
    keys(&mut p, "*85555", t0);
    keys(&mut p, "001", t0);
    keys(&mut p, "010541", t0);
    keys(&mut p, "#", t0);
    keys(&mut p, "#", t0);
    assert_eq!(p.cfg(3).kind, ZoneKind::H24(H24::CarbonMonoxide));
    p.set_zone(3, true, false, false, t0 + secs(1));
    p.tick(t0 + secs(2), 0);
    assert_eq!(p.bell_sent, BellPattern::Temporal4);

    // Synth Temporal4 stays finite & audible during pulse burst and silent during 5s pause.
    let mut s = crate::audio::synth::Synth::new(48_000.0);
    s.apply(AudioCmd::Bell(BellPattern::Temporal4));
    let burst_peak = (0..4_800).map(|_| s.next().abs()).fold(0.0f32, f32::max);
    assert!(burst_peak > 0.01 && burst_peak <= 1.0);

    // PK5500 BuzzerLevel 0 mutes keypad beeps; 1..20 produces audible beeps.
    let mut sb = crate::audio::synth::Synth::new(48_000.0);
    sb.apply(AudioCmd::BuzzerLevel(0));
    sb.apply(AudioCmd::Beeps { count: 1, on_ms: 80, off_ms: 0 });
    let muted_peak = (0..3_000).map(|_| sb.next().abs()).fold(0.0f32, f32::max);
    assert!(muted_peak < 0.005, "level 0 should mute PK5500 buzzer (got {})", muted_peak);
    sb.apply(AudioCmd::BuzzerLevel(18));
    sb.apply(AudioCmd::Beeps { count: 1, on_ms: 80, off_ms: 0 });
    let loud_peak = (0..3_000).map(|_| sb.next().abs()).fold(0.0f32, f32::max);
    assert!(loud_peak > 0.05);
}

#[test]
fn pc1832_manual_sections_and_rules_work() {
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1832);
    // Exit delay termination keeps the arming below short (see armed_away).
    p.ee.sections.insert("014".to_string(), "67".to_string());

    // [900] Version display
    keys(&mut p, "*85555", t0);
    keys(&mut p, "900", t0);
    let lcd = p.lcd_1832(t0, 0);
    assert!(lcd.lines[1].contains("0460"), "expected 0460 version, got {:?}", lcd.lines);

    // [008] Maintenance code arms but cannot disarm unless in alarm
    keys(&mut p, "0087777", t0);
    // [377] Swinger limit = 002, tx delay = 015
    keys(&mut p, "377002003003015", t0);
    keys(&mut p, "#", t0);
    // [989] Default master code check after changing master
    keys(&mut p, "0074321", t0);
    assert!(p.ee.code_matches("40", "4321"));
    keys(&mut p, "9895555", t0);
    assert!(p.ee.code_matches("40", "1234"), "[989] should default master code to 1234");
    keys(&mut p, "#", t0);

    assert_eq!(p.swinger_limit(), 2);
    assert_eq!(p.tx_delay(), 15);

    // Arm with maintenance code, leave through door
    keys(&mut p, "7777", t0 + secs(1));
    assert!(p.exit.is_some());
    p.set_zone(1, true, false, false, t0 + secs(3));
    p.set_zone(1, false, false, false, t0 + secs(5));
    p.tick(t0 + secs(11), 0);
    assert!(p.armed.is_some());

    // Maintenance code cannot disarm while no alarm is sounding
    keys(&mut p, "7777", t0 + secs(15));
    assert!(p.armed.is_some(), "maintenance code must not disarm when not in alarm");

    // Master code disarms normally
    keys(&mut p, "1234", t0 + secs(16));
    assert!(p.armed.is_none());

    // [*][1] [95] Save bypass group and [91] Recall saved bypass group
    keys(&mut p, "*10395#", t0 + secs(20));
    assert!(p.zones[2].bypassed);
    keys(&mut p, "*100#", t0 + secs(21));
    assert!(!p.zones[2].bypassed);
    keys(&mut p, "*191#", t0 + secs(22));
    assert!(p.zones[2].bypassed, "[91] should recall saved bypass group");
}

#[test]
fn pc1550_manual_fire_pattern_and_maids_code() {
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1550);

    // By default Section [32] Light 6 is ON -> Standard Pulsed fire bell.
    p.set_zone(6, true, false, false, t0);
    p.tick(t0 + secs(1), 0);
    assert_eq!(p.bell_sent, BellPattern::Pulsed);
    keys(&mut p, "1234", t0 + secs(2));
    p.set_zone(6, false, false, false, t0 + secs(3));
    p.tick(t0 + secs(4), 0);

    // Turn Section [32] Light 6 OFF -> Temporal 3 fire bell (per PC1550 v4.1 manual).
    keys(&mut p, "*81500326##", t0 + secs(5));
    p.set_zone(6, true, false, false, t0 + secs(6));
    p.tick(t0 + secs(7), 0);
    assert_eq!(p.bell_sent, BellPattern::Temporal3);
    keys(&mut p, "1234", t0 + secs(8));
    p.set_zone(6, false, false, false, t0 + secs(9));

    // Program User 6 = 6666 and enable Section [13] Light 5 (Maid's one-time code).
    keys(&mut p, "*5123466666#", t0 + secs(10));
    assert!(p.ee.code_matches("6", "6666"));
    keys(&mut p, "*81500135##", t0 + secs(11));
    keys(&mut p, "6666", t0 + secs(12));
    assert!(p.exit.is_some());
    assert!(!p.ee.has_code("6"), "Maid's code 6 should clear itself after arming");
}

#[test]
fn face_descriptor_tolerates_brightness_shift() {
    use crate::vision::faces::{chi2, describe, FaceRect};
    let (w, h) = (96usize, 96usize);
    let mut img_a = vec![0u8; w * h];
    let mut img_b = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let v = (((x * 7 + y * 13) % 120) as f32 + ((x as f32 - 48.0).hypot(y as f32 - 48.0) * 1.4)) as u8;
            img_a[y * w + x] = v.clamp(20, 180);
            img_b[y * w + x] = (v.saturating_add(45)).clamp(40, 240);
        }
    }
    let r = FaceRect { x: 16.0, y: 16.0, w: 64.0, h: 64.0, votes: 5 };
    let da = describe(&img_a, w, h, &r).expect("desc a");
    let db = describe(&img_b, w, h, &r).expect("desc b");
    let dist = chi2(&da, &db);
    assert!(dist < 2.5, "brightness-shifted face should have low chi2 distance, got {}", dist);
}

#[test]
fn keypress_dips_steady_entry_tone_and_disarm_beeps_rapidly() {
    // 1. Synth KeyBeep cleanly interrupts a steady entry-delay tone with a silent gap.
    let mut s = crate::audio::synth::Synth::new(48_000.0);
    s.apply(AudioCmd::Sounder(Pattern::Steady));
    for _ in 0..4_800 {
        s.next();
    }
    s.apply(AudioCmd::KeyBeep);
    // Let piezo resonator ring down (~12ms) inside the 28ms dip window.
    for _ in 0..800 {
        s.next();
    }
    let dip_peak = (0..400).map(|_| s.next().abs()).fold(0.0f32, f32::max);
    assert!(dip_peak < 0.02, "keypress should create an audible dip during steady entry tone, got {}", dip_peak);

    // 2. Disarming during entry delay emits 3 rapid confirmation beeps.
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1832);
    armed_away(&mut p, t0);
    p.set_zone(1, true, false, false, t0 + secs(20));
    assert!(p.entry.is_some());
    p.out.clear();
    keys(&mut p, "1234", t0 + secs(22));
    assert!(p.armed.is_none());
    assert!(
        p.out.iter().any(|o| matches!(o, Out::Audio(AudioCmd::Beeps { count: 3, .. }))),
        "disarming should emit 3 rapid confirmation beeps"
    );

    // 3. PK5500 LCD automatically cycles between Ready and Date/Time when idle.
    let p_idle = panel(Model::Pc1832);
    let now = Instant::now();
    let s0 = p_idle.lcd_1832(now, 1_700_000_000_000);
    let s1 = p_idle.lcd_1832(now + secs(5), 1_700_000_005_000);
    assert_ne!(s0.lines, s1.lines, "PK5500 display should auto-cycle between Ready and Date/Time");
}

#[test]
fn non_person_shapes_and_global_lighting_shifts_are_filtered() {
    use crate::vision::motion::Motion;
    use crate::vision::track::find_blobs;

    // 1. Flat wide horizontal strip (like a TV bar or shadow edge) is rejected as non-person.
    let (w, h) = (160usize, 120usize);
    let mut mask = vec![0u8; w * h];
    for y in 40..46 {
        for x in 20..120 {
            mask[y * w + x] = 1;
        }
    }
    let blobs = find_blobs(&mask, w, h, 80);
    assert!(blobs.is_empty(), "flat horizontal bar should not be detected as a person");

    // 2. Person-sized upright blob passes the shape filter.
    let mut mask2 = vec![0u8; w * h];
    for y in 30..90 {
        for x in 65..85 {
            mask2[y * w + x] = 1;
        }
    }
    let blobs2 = find_blobs(&mask2, w, h, 80);
    assert_eq!(blobs2.len(), 1, "upright person-sized blob should be kept");

    // 3. Mild global brightness shift (webcam auto-exposure) does not trigger false motion.
    let mut m = Motion::new();
    let frame1 = vec![90u8; w * h];
    let frame2 = vec![108u8; w * h]; // uniform +18 brightness shift across entire room
    m.process(&frame1, w, h, 3, None, &[]);
    let out = m.process(&frame2, w, h, 3, None, &[]);
    assert!(out.total_frac < 0.005, "uniform brightness shift should not look like motion, got {}", out.total_frac);
}

#[test]
fn only_real_human_bodies_count_as_people() {
    let walker = crate::vision::demo::people(5.0);
    assert_eq!(walker.len(), 1);
    assert!(walker[0].humanoid(0.5));
    assert!(crate::vision::demo::people(1.0).is_empty());
    // Back turned or face out of view: shoulders, elbows and hips are enough.
    let mut torso = walker[0];
    for i in [0, 1, 2, 3, 4, 13, 14, 15, 16] {
        torso.kp[i].2 = 0.0;
    }
    assert!(torso.humanoid(0.5));
    // A confident box without a human skeleton (furniture, a poster) is not a person.
    let mut thing = walker[0];
    for k in thing.kp.iter_mut() {
        k.2 = 0.05;
    }
    assert!(!thing.humanoid(0.5));
    // A weak detection is not a person even with joints.
    let mut weak = walker[0];
    weak.score = 0.35;
    assert!(!weak.humanoid(0.5));
    // Faces are only looked for where the head can be.
    let head = walker[0].head_region();
    assert!(head.contains(walker[0].kp[0].0, walker[0].kp[0].1));
    assert!(!head.contains(walker[0].kp[15].0, walker[0].kp[15].1));
}

#[test]
fn the_body_detector_sees_nobody_in_an_empty_room() {
    let net = crate::vision::pose::PoseNet::load().expect("body models load");
    let empty = crate::vision::demo::scene(1.0);
    assert!(net.detect(&empty).iter().all(|p| !p.humanoid(0.5)));
    let mut s = 12_345u32;
    let noise: Vec<u8> = (0..480 * 360 * 3)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s >> 24) as u8
        })
        .collect();
    let img = crate::vision::frame::Rgb { w: 480, h: 360, data: noise };
    assert!(net.detect(&img).iter().all(|p| !p.humanoid(0.5)));
}

#[test]
fn the_audio_output_never_goes_digitally_silent() {
    let mut s = crate::audio::synth::Synth::new(48_000.0);
    for _ in 0..1_000 {
        let v = s.next();
        assert!(v != 0.0 && v.abs() < 1e-3);
    }
}

// ----- RoomGuard 2.1 -----

#[test]
fn stay_reactivation_waits_for_the_exit_delay() {
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1832);
    // Arm with the code and don't leave: it arms Stay with zone 2 bypassed.
    keys(&mut p, "1234", t0);
    p.tick(t0 + secs(121), 0);
    assert!(matches!(p.armed, Some(a) if a.mode == ArmMode::Stay));
    assert!(p.zones[1].bypassed);
    // [*][1] switches to Away, but the zone only comes back after the exit delay.
    let t1 = t0 + secs(130);
    keys(&mut p, "*1", t1);
    p.tick(t1 + secs(1), 0);
    assert!(p.zones[1].bypassed, "still bypassed during the delay");
    p.tick(t1 + secs(121), 0);
    assert!(!p.zones[1].bypassed, "back after the exit delay");
}

#[test]
fn holding_a_digit_works_like_a_function_key() {
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1832);
    p.key(Key::Digit(1), true, t0);
    p.tick(t0 + Duration::from_millis(2100), 0);
    assert!(matches!(p.exit, Some(e) if e.how == ArmHow::StayKey), "holding 1 is the Stay key");
    assert!(p.buf.is_empty());
    // A quick press is just a digit.
    let mut p = panel(Model::Pc1832);
    p.key(Key::Digit(1), true, t0);
    p.key(Key::Digit(1), false, t0 + Duration::from_millis(150));
    p.tick(t0 + secs(3), 0);
    assert!(p.exit.is_none());
}

#[test]
fn pc1832_exit_delay_beeps_only_with_section_014_option_6() {
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1832);
    p.key(Key::Away, true, t0);
    p.tick(t0 + Duration::from_millis(2100), 0);
    assert!(p.exit.is_some());
    p.tick(t0 + secs(5), 0);
    assert!(matches!(p.sounder_sent, Pattern::Beeps { count: 1, period_ms: 996, .. }), "beeps once a second");
    p.tick(t0 + secs(115), 0);
    assert!(matches!(p.sounder_sent, Pattern::Beeps { count: 3, period_ms: 996, .. }), "a burst of three each second at the end");
    let mut p = panel(Model::Pc1832);
    p.ee.sections.insert("014".to_string(), String::new());
    p.key(Key::Away, true, t0);
    p.tick(t0 + Duration::from_millis(2100), 0);
    p.tick(t0 + secs(5), 0);
    assert_eq!(p.sounder_sent, Pattern::Off, "silent exit delay");
}

#[test]
fn a_pc1550_is_silent_during_exit_delay_from_the_factory() {
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1550);
    keys(&mut p, "1234", t0);
    p.tick(t0 + secs(5), 0);
    assert_eq!(p.sounder_sent, Pattern::Off, "[34] light 1 is off from the factory");
    let mut p = panel(Model::Pc1550);
    p.ee.sections.insert("34".to_string(), "1".to_string());
    keys(&mut p, "1234", t0);
    p.tick(t0 + secs(5), 0);
    assert!(matches!(p.sounder_sent, Pattern::Beeps { .. }));
}

#[test]
fn old_saves_get_the_factory_defaults_once() {
    let mut ee = crate::engine::eeprom::Eeprom::factory(Model::Pc1832);
    ee.rev = 0;
    ee.sections.insert("013".to_string(), "8".to_string());
    ee.sections.insert("014".to_string(), "1".to_string());
    let fixed = ee.migrate(Model::Pc1832);
    assert_eq!(ee.sections["013"], "3567");
    assert_eq!(ee.sections["014"], "1", "your own programming is kept");
    assert!(fixed.contains(&"013"));
    assert!(ee.migrate(Model::Pc1832).is_empty(), "only runs once");
    let fresh = crate::engine::eeprom::Eeprom::factory(Model::Pc1550);
    assert_eq!(fresh.rev, crate::engine::eeprom::EEPROM_REV);
}

#[test]
fn zone_attributes_follow_the_manual_table() {
    use crate::engine::eeprom::Eeprom;
    assert_eq!(Eeprom::default_zone_attrs(5), "12456", "stay/away zones can be force armed");
    assert_eq!(Eeprom::default_zone_attrs(8), "1", "fire zones are audible only");
    assert_eq!(Eeprom::default_zone_attrs(3), "12346");
    let p = panel(Model::Pc1832);
    assert!(p.cfg(2).force_arm, "zone 2, the room, can be force armed");
}

#[test]
fn loop_response_matches_each_panel() {
    let p = panel(Model::Pc1550);
    assert_eq!(p.loop_response_ms(1), 500);
    let p = panel(Model::Pc1832);
    assert_eq!(p.loop_response_ms(1), 400);
}

#[test]
fn a_queued_save_reads_back_right_away() {
    setup();
    let path = crate::util::data_dir().join("saver-test.json");
    assert!(crate::util::save_json(&path, &vec![1u32, 2, 3]));
    let back: Vec<u32> = crate::util::load_json(&path).expect("reads back");
    assert_eq!(back, vec![1, 2, 3]);
    assert!(crate::util::flush_saves(Duration::from_secs(5)));
    assert!(path.exists());
}

#[test]
fn the_last_digit_beeps_before_the_confirmation_beeps() {
    let mut s = crate::audio::synth::Synth::new(48_000.0);
    s.apply(AudioCmd::KeyBeep);
    s.apply(AudioCmd::Beeps { count: 3, on_ms: 80, off_ms: 80 });
    let first: f32 = (0..2_400).map(|_| s.next().abs()).fold(0.0, f32::max);
    assert!(first > 0.05, "the keypress beep sounds");
    // Keypress then three beeps puts the third beep at about 425 to 505 ms.
    for _ in 2_400..21_120 {
        s.next();
    }
    let late: f32 = (0..2_880).map(|_| s.next().abs()).fold(0.0, f32::max);
    assert!(late > 0.05, "the three beeps follow the keypress instead of replacing it");
}

#[test]
fn body_evidence_weighs_the_detector_against_the_skeleton() {
    use crate::vision::pose::Evidence;
    let base = crate::vision::demo::people(5.0)[0];
    assert_eq!(base.evidence(0.45), Evidence::Strong);
    // A coat on a hook: a middling score and a shoulderless skeleton.
    let mut coat = base;
    coat.score = 0.37;
    for (i, k) in coat.kp.iter_mut().enumerate() {
        k.2 = if i >= 13 { 0.58 } else { 0.35 };
    }
    assert!(coat.evidence(0.38) < Evidence::Normal);
    // Someone far away: a weaker score but a clear skeleton.
    let mut far = base;
    far.score = 0.47;
    assert_eq!(far.evidence(0.45), Evidence::Normal);
}

#[test]
fn a_body_that_never_moves_is_never_a_person() {
    use crate::vision::track::{Blob, Tracker};
    let t0 = Instant::now();
    let mut tr = Tracker::new();
    tr.body_mode = true;
    tr.confirm_hits = 3;
    for i in 0..30u64 {
        tr.update(&[Blob { cx: 0.5, cy: 0.5, w: 0.1, h: 0.3, px: 100, strength: 3, motion: 0.0 }], t0 + Duration::from_millis(i * 100));
    }
    assert!(tr.tracks.iter().all(|t| !t.confirmed), "a poster or a coat stays unconfirmed");
    let mut tr = Tracker::new();
    tr.body_mode = true;
    tr.confirm_hits = 3;
    for i in 0..10u64 {
        let x = 0.2 + i as f32 * 0.03;
        tr.update(&[Blob { cx: x, cy: 0.5, w: 0.1, h: 0.3, px: 100, strength: 3, motion: 0.2 }], t0 + Duration::from_millis(i * 100));
    }
    assert!(tr.tracks.iter().any(|t| t.confirmed), "someone walking in is confirmed");
}

#[test]
fn a_weak_sighting_never_starts_a_person() {
    use crate::vision::track::{Blob, Tracker};
    let t0 = Instant::now();
    let mut tr = Tracker::new();
    tr.body_mode = true;
    tr.update(&[Blob { cx: 0.5, cy: 0.5, w: 0.1, h: 0.3, px: 100, strength: 1, motion: 0.3 }], t0);
    assert!(tr.tracks.is_empty());
}

#[test]
fn the_pc1550_shows_camera_troubles_with_light_and_beeps() {
    use crate::engine::panel::tb;
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1550);
    p.inputs.cam_enabled = true;
    p.inputs.cam_covered = true;
    p.tick(t0, 0);
    p.tick(t0 + secs(1), 0);
    assert!(p.troubles & tb::ZONE_TAMPER != 0);
    assert_eq!(p.leds_1550(t0 + secs(1))[4], Led::On, "the Trouble light comes on");
    assert!(matches!(p.sounder_sent, Pattern::Beeps { count: 2, .. }), "two beeps every 10 seconds");
    keys(&mut p, "#", t0 + secs(2));
    p.tick(t0 + secs(3), 0);
    assert_eq!(p.sounder_sent, Pattern::Off, "# silences the beeps");
    assert_eq!(p.leds_1550(t0 + secs(3))[4], Led::On, "the light stays until it's fixed");
    keys(&mut p, "*2", t0 + secs(4));
    assert_eq!(p.leds_1550(t0 + secs(4))[5], Led::Flash, "zone light 1 flashes for its camera");
    p.inputs.cam_covered = false;
    keys(&mut p, "#", t0 + secs(5));
    p.tick(t0 + secs(6), 0);
    assert_eq!(p.leds_1550(t0 + secs(6))[4], Led::Off, "fixed, the light goes out");
}

#[test]
fn entry_tone_stops_when_burglary_bell_starts_and_pk5500_ready_clears_on_exit() {
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1832);
    assert_eq!(p.leds_5500(t0)[0], Led::On, "Ready LED is on when disarmed");
    assert_eq!(p.leds_5500(t0)[1], Led::Off, "Armed LED is off when disarmed");

    // Arming starts exit delay: PK5500 Ready turns OFF immediately, Armed turns ON.
    keys(&mut p, "1234", t0);
    assert!(p.exit.is_some());
    let ex_leds = p.leds_5500(t0 + secs(1));
    assert_eq!(ex_leds[0], Led::Off, "Ready LED turns off during exit delay");
    assert_eq!(ex_leds[1], Led::On, "Armed LED turns on during exit delay");

    // Leave through door and let exit delay finish, then trip delay zone for entry delay.
    p.set_zone(1, true, false, false, t0 + secs(5));
    p.set_zone(1, false, false, false, t0 + secs(7));
    p.tick(t0 + secs(121), 0);
    assert!(p.armed.is_some());

    p.set_zone(1, true, false, false, t0 + secs(130));
    p.tick(t0 + secs(131), 0);
    assert_eq!(p.sounder_sent, Pattern::Steady, "entry delay sounds continuous keypad tone");
    assert_eq!(p.bell_sent, BellPattern::Off);

    // Entry delay expires: external bell sounds, keypad piezo stops (since [018] opt 5 is OFF).
    p.tick(t0 + secs(161), 0);
    assert_eq!(p.bell_sent, BellPattern::Steady, "external siren/bell triggers");
    assert_eq!(p.sounder_sent, Pattern::Off, "keypad piezo stops when burglary siren takes over");

    // Turning on [018] Option 5 (Keypad Buzzer Follows Bell) makes the keypad buzzer follow the bell.
    p.ee.sections.insert("018".to_string(), "5".to_string());
    p.tick(t0 + secs(162), 0);
    assert_eq!(p.sounder_sent, Pattern::Steady, "[018] opt 5 makes keypad buzzer follow bell");

    // Disarming restores Ready LED and turns off Armed LED.
    p.set_zone(1, false, false, false, t0 + secs(163));
    keys(&mut p, "1234", t0 + secs(164));
    p.tick(t0 + secs(165), 0);
    assert_eq!(p.leds_5500(t0 + secs(165))[0], Led::On);
    assert_eq!(p.leds_5500(t0 + secs(165))[1], Led::Off);
}

#[test]
fn vista20p_commands_autostay_twostep_clear_and_voice_rom() {
    use crate::audio::voice;
    // 1. 6160V Voice ROM whitelist filters out unsupported words like PANEL/TAMPER
    assert_eq!(voice::filter_zone_words("BEDROOM DOOR"), vec!["bedroom", "door"]);
    assert!(voice::filter_zone_words("PANEL TAMPER").is_empty());

    // 2. VoicePlayer synthesizes audible non-zero audio for ROM tokens
    let mut vp = voice::VoicePlayer::new(48_000.0);
    vp.speak(vec!["disarmed".into(), "ready_to_arm".into()]);
    assert!(vp.is_speaking());
    let peak = (0..12_000).map(|_| vp.next().abs()).fold(0.0f32, f32::max);
    assert!(peak > 0.05, "6160V voice ROM should produce audible speech (got {})", peak);

    // 3. VISTA-20P [Code] + 2 (AWAY) -> Exit Delay -> Auto-Stay (*84) if no door opened
    let t0 = Instant::now();
    let mut p = panel(Model::Vista20p);
    keys(&mut p, "12342", t0);
    assert!(p.exit.is_some());
    assert!(p.lcd_vista(t0, 0).lines[0].contains("AWAY"));
    // Advance past 60s exit delay without opening Zone 1 -> Auto-Stay converts to STAY!
    p.tick(t0 + secs(65), 0);
    assert_eq!(p.armed.map(|a| a.mode), Some(ArmMode::Stay));

    // 4. Disarm with [Code] + 1 (OFF), then arm AWAY and open+close Zone 1 during exit -> stays AWAY
    keys(&mut p, "12341", t0 + secs(66));
    assert!(p.armed.is_none());
    keys(&mut p, "12342", t0 + secs(70));
    p.set_zone(1, true, false, false, t0 + secs(75));
    p.set_zone(1, false, false, false, t0 + secs(78));
    p.tick(t0 + secs(135), 0);
    assert_eq!(p.armed.map(|a| a.mode), Some(ArmMode::Away));

    // 5. Trip Zone 3 (Perimeter) -> instant alarm, "LOUD, CONTINUOUS" at the keypad, two-step clear
    p.set_zone(3, true, false, false, t0 + secs(140));
    p.tick(t0 + secs(141), 0);
    assert!(p.alarm_sounding(t0 + secs(141)));
    assert_eq!(p.sounder_sent, Pattern::HiLo { hi_hz: 1360, lo_hz: 1019, step_ms: 505 }, "the measured two-tone warble");
    // 1st [Code] + 1 silences alarm but leaves alarm memory latched on LCD
    keys(&mut p, "12341", t0 + secs(145));
    p.set_zone(3, false, false, false, t0 + secs(146));
    assert!(p.vista_alarm_mem_zone.is_some());
    // The first code + OFF leaves the alarm on the display, as on a real 6160.
    assert!(p.lcd_vista(t0 + secs(146), 0).lines[0].contains("ALARM 03"));
    // The second shows "Alarm Canceled" for a moment (*86), then Ready to Arm.
    keys(&mut p, "12341", t0 + secs(148));
    assert!(p.vista_alarm_mem_zone.is_none());
    assert!(p.lcd_vista(t0 + secs(148), 0).lines[0].contains("Alarm Canceled"));
    assert!(p.lcd_vista(t0 + secs(150), 0).lines[1].contains("Ready to Arm"));

    // 6. Installer programming: 4112 + 800 -> *35 -> 45 -> *99
    keys(&mut p, "4112800", t0 + secs(155));
    assert!(p.is_programming());
    keys(&mut p, "*3545*99", t0 + secs(156));
    assert!(!p.is_programming());
    assert_eq!(p.entry_secs(ZoneKind::Delay1), 45);

    // 7. Confirmation beep plays cleanly BEFORE 6160V voice speaks (not ducked/swallowed)
    let mut s = crate::audio::synth::Synth::new(48_000.0);
    s.apply(AudioCmd::Voice(crate::audio::KeypadVoice::Vista6160));
    s.apply(AudioCmd::Beeps { count: 1, on_ms: 220, off_ms: 80 });
    s.apply(AudioCmd::Speak(vec!["disarmed".into()]));
    // First 200 ms: confirmation beep must be loud and clear
    let beep_peak = (0..9_600).map(|_| s.next().abs()).fold(0.0f32, f32::max);
    assert!(beep_peak > 0.10, "6160V confirmation beep must not be swallowed by voice (got {})", beep_peak);
    // Skip past the remaining 20 ms beep + 80 ms gap, then voice must speak audibly
    for _ in 0..5_200 {
        s.next();
    }
    let voice_peak = (0..12_000).map(|_| s.next().abs()).fold(0.0f32, f32::max);
    assert!(voice_peak > 0.05, "6160V voice must speak right after confirmation beep (got {})", voice_peak);
}

#[test]
fn all_three_keypads_render_without_panicking() {
    use crate::keypads::{classic, lcd, vista, KeyInput};
    use eframe::egui;

    let ctx = egui::Context::default();
    crate::theme::install(&ctx);
    let audio = crate::audio::start();
    let (tx, rx) = std::sync::mpsc::channel();
    let cam = crate::vision::start(crate::config::CameraCfg { enabled: false, ..Default::default() }, tx.clone(), ctx.clone());
    let notify = crate::notify::start(tx.clone());
    let engine = crate::engine::start(crate::config::AppConfig::default(), tx, rx, audio.clone(), cam, notify, ctx.clone());
    let mut input = KeyInput::default();
    let mut view = crate::engine::View::default();
    view.lcd = ["****DISARMED****".into(), "Ready to Arm    ".into()];
    view.backlight = 1.0;

    for open in [true, false] {
        input.door_anim_1550 = if open { 1.0 } else { 0.0 };
        input.door_anim_5500 = if open { 1.0 } else { 0.0 };
        input.door_anim_6160 = if open { 1.0 } else { 0.0 };
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500.0, 500.0));
                classic::draw(ui, area, &view, &mut input, &engine, &audio, 1.0, 0.5);
                lcd::draw(ui, area, &view, &mut input, &engine, &audio, 1.0, 0.5);
                vista::draw(ui, area, &view, &mut input, &engine, &audio, 1.0, 0.5);
            });
        });
    }
    engine.shutdown();
}

#[test]
fn the_vista_follows_its_user_guide() {
    let t0 = Instant::now();
    let mut p = panel(Model::Vista20p);
    // Quick arm (*21) and quick bypass (*23) are off from the factory.
    keys(&mut p, "#2", t0);
    assert!(p.exit.is_none(), "quick arm stays off until *21 is set");

    // Code + STAY + STAY is NIGHT-STAY.
    keys(&mut p, "123433", t0 + secs(10));
    assert!(p.vista_night);
    assert!(p.lcd_vista(t0 + secs(10), 0).lines[0].contains("NIGHT-STAY"));
    keys(&mut p, "12341", t0 + secs(20));
    assert!(p.armed.is_none() && p.exit.is_none() && !p.vista_night);

    // Bypassing only adds, and code + OFF while disarmed removes it.
    keys(&mut p, "1234603", t0 + secs(30));
    assert!(p.zones[2].bypassed);
    keys(&mut p, "1234603", t0 + secs(40));
    assert!(p.zones[2].bypassed, "entering a bypassed zone again keeps it bypassed");
    keys(&mut p, "12341", t0 + secs(50));
    assert!(!p.zones[2].bypassed);

    // In the entry delay a code alone quiets the keypad and disarms 15 s later.
    keys(&mut p, "12342", t0 + secs(60));
    p.tick(t0 + secs(125), 0);
    assert!(p.armed.is_some());
    p.set_zone(1, true, false, false, t0 + secs(130));
    p.tick(t0 + secs(130), 0);
    assert!(p.entry.is_some());
    assert!(matches!(p.sounder_sent, Pattern::Beeps { on_ms: 500, period_ms: 1010, .. }), "slow entry beeping");
    keys(&mut p, "1234", t0 + secs(132));
    p.tick(t0 + secs(133), 0);
    assert_eq!(p.sounder_sent, Pattern::Off, "the entry beeps stop once a code is in");
    p.tick(t0 + secs(148), 0);
    assert!(p.armed.is_none(), "a valid code disarms about 15 seconds later without OFF");

    // A trouble beeps rapidly until any key is pressed.
    p.inputs.cam_enabled = true;
    p.inputs.cam_covered = true;
    p.tick(t0 + secs(150), 0);
    assert!(matches!(p.sounder_sent, Pattern::Beeps { period_ms: 160, .. }), "rapid trouble beeping");
    keys(&mut p, "5", t0 + secs(151));
    p.tick(t0 + secs(152), 0);
    assert_eq!(p.sounder_sent, Pattern::Off, "any key silences it");
}

#[test]
fn the_vista_keypad_plays_the_measured_notes() {
    use crate::audio::{synth::Synth, AudioCmd, KeypadVoice};
    let sr = 48_000.0;
    let mut s = Synth::new(sr);
    s.apply(AudioCmd::Voice(KeypadVoice::Vista6160));
    s.apply(AudioCmd::Sounder(Pattern::HiLo { hi_hz: 1360, lo_hz: 1019, step_ms: 505 }));
    let n = (0.505 * sr) as usize;
    let hi: Vec<f32> = (0..n).map(|_| s.next()).collect();
    let lo: Vec<f32> = (0..n).map(|_| s.next()).collect();
    let pitch = |v: &[f32]| {
        let w = &v[v.len() / 4..v.len() * 3 / 4];
        let ups = w.windows(2).filter(|p| p[0] <= 0.0 && p[1] > 0.0).count();
        ups as f32 / (w.len() as f32 / sr)
    };
    assert!((pitch(&hi) - 1360.0).abs() < 15.0, "high note {}", pitch(&hi));
    assert!((pitch(&lo) - 1019.0).abs() < 15.0, "low note {}", pitch(&lo));
    let peak = hi.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak > 0.1, "the alarm is loud ({})", peak);
}

#[test]
fn the_pc1832_sounds_like_a_real_powerseries_keypad() {
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1832);
    p.out.clear();
    keys(&mut p, "1234", t0);
    assert!(p.exit.is_some());
    assert!(
        p.out.iter().any(|o| matches!(o, Out::Audio(AudioCmd::Beeps { count: 6, on_ms: 67, off_ms: 50 }))),
        "a code arms it with six quick beeps"
    );
    p.tick(t0 + secs(125), 0);
    assert!(p.armed.is_some());
    p.set_zone(1, true, false, false, t0 + secs(130));
    p.tick(t0 + secs(131), 0);
    assert_eq!(p.sounder_sent, Pattern::Steady, "a steady entry tone");
    p.tick(t0 + secs(155), 0);
    assert_eq!(
        p.sounder_sent,
        Pattern::ShortShortLong { short_ms: 94, gap_ms: 26, long_ms: 731 },
        "then beep, beep, beeeep for the last ten seconds"
    );
}

#[test]
fn the_vista_entry_delay_beeps_faster_at_the_end() {
    let t0 = Instant::now();
    let mut p = panel(Model::Vista20p);
    keys(&mut p, "12342", t0);
    p.tick(t0 + secs(65), 0);
    assert!(p.armed.is_some());
    p.set_zone(1, true, false, false, t0 + secs(70));
    p.tick(t0 + secs(71), 0);
    assert!(matches!(p.sounder_sent, Pattern::Beeps { on_ms: 500, period_ms: 1010, .. }), "slow at first");
    p.tick(t0 + secs(92), 0);
    assert!(matches!(p.sounder_sent, Pattern::Beeps { on_ms: 90, period_ms: 160, .. }), "rapid for the last ten seconds");
}

#[test]
fn pc1550_entry_delay_pulses_in_last_10_seconds_and_vista_speaks_live_ready() {
    // 1. PC1550RK pulses (beep, beep, beeeep) in the last 10 seconds of entry delay.
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1550);
    keys(&mut p, "1234", t0);
    p.tick(t0 + secs(125), 0);
    assert!(p.armed.is_some());
    p.set_zone(1, true, false, false, t0 + secs(130));
    p.tick(t0 + secs(131), 0);
    assert_eq!(p.sounder_sent, Pattern::Steady, "steady entry tone before last 10 seconds");
    p.tick(t0 + secs(155), 0);
    assert_eq!(
        p.sounder_sent,
        Pattern::ShortShortLong { short_ms: 94, gap_ms: 26, long_ms: 731 },
        "PC1550RK should pulse beep-beep-beeeep in the last 10 seconds"
    );

    // 2. Vista20p evaluates ready() LIVE when speaking after disarming (so a
    // closing doorway doesn't falsely announce "not ready to arm" when ready).
    let mut v = panel(Model::Vista20p);
    keys(&mut v, "12342", t0);
    v.tick(t0 + secs(65), 0);
    v.set_zone(1, true, false, false, t0 + secs(70));
    v.tick(t0 + secs(70), 0);
    v.out.clear();
    keys(&mut v, "12341", t0 + secs(71));
    // Zone 1 closes 0.5s later, well before the 2.19s delayed voice announcement.
    v.set_zone(1, false, false, false, t0 + Duration::from_millis(71_500));
    v.tick(t0 + Duration::from_millis(73_500), 0);
    let spoke_ready = v.out.iter().any(|o| matches!(o, Out::Audio(AudioCmd::Speak(toks)) if toks.iter().any(|t| t == "ready_to_arm")));
    let spoke_not_ready = v.out.iter().any(|o| matches!(o, Out::Audio(AudioCmd::Speak(toks)) if toks.iter().any(|t| t == "not_ready_to_arm")));
    assert!(spoke_ready && !spoke_not_ready, "Vista must speak ready_to_arm when zone closed before announcement");

    // 3. Vista20p [Code] + 6 shows BYPASS ZONE and stays open past 3 seconds.
    keys(&mut v, "12346", t0 + secs(80));
    v.tick(t0 + secs(83), 0);
    assert!(v.lcd_vista(t0 + secs(83), 0).lines[0].contains("BYPASS"));
    keys(&mut v, "03", t0 + secs(84));
    assert!(v.zones[2].bypassed);

    // 4. Switching between all 4 sirens during Steady & Temporal3 produces loud audio,
    // and PC1832 Panic key sounds the bell out of the box.
    let mut s = crate::audio::synth::Synth::new(48_000.0);
    s.apply(AudioCmd::Volumes { master: 0.8, siren: 0.45, keypad: 0.7, line: 0.0 });
    for sm in SirenModel::ALL {
        s.apply(AudioCmd::Siren { model: sm, warble: true });
        s.apply(AudioCmd::Bell(BellPattern::Steady));
        // Let motor sirens / bells spin up (~0.6s) and measure peak
        let peak = (0..36_000).map(|_| s.next().abs()).fold(0.0f32, f32::max);
        assert!(peak > 0.15, "siren {:?} must sound loudly on steady alarm (got {})", sm, peak);
    }
    let mut p18 = panel(Model::Pc1832);
    p18.key(Key::Panic, true, t0);
    p18.tick(t0 + Duration::from_millis(2100), 0);
    assert_eq!(p18.bell_sent, BellPattern::Steady, "PC1832 Panic key should sound the siren out of the box");
}

#[test]
fn the_vista_stay_exit_delay_is_quiet_and_away_beeps_faster_at_the_end() {
    let t0 = Instant::now();
    let mut p = panel(Model::Vista20p);
    keys(&mut p, "12343", t0);
    p.tick(t0 + secs(2), 0);
    assert!(p.exit.is_some());
    assert_eq!(p.sounder_sent, Pattern::Off, "STAY gives three beeps, then a quiet exit delay");
    keys(&mut p, "12341", t0 + secs(10));
    assert!(p.exit.is_none() && p.armed.is_none());
    keys(&mut p, "12342", t0 + secs(20));
    p.tick(t0 + secs(22), 0);
    assert!(matches!(p.sounder_sent, Pattern::Beeps { on_ms: 500, period_ms: 1010, .. }), "AWAY gets the slow exit warning");
    p.tick(t0 + secs(72), 0);
    assert!(matches!(p.sounder_sent, Pattern::Beeps { on_ms: 90, period_ms: 160, .. }), "rapid for the last ten seconds");
}

#[test]
fn the_sirens_play_their_measured_pitches() {
    let sr = 48_000.0;
    // Pitch from the autocorrelation peak between half and 1.5 times the
    // expected pitch. Strong harmonics can't fool it the way they fool zero
    // crossings (the Master Blaster crosses zero three times a cycle).
    let pitch = |model: SirenModel, warble: bool, skip_s: f32, len_s: f32, expect: f32| {
        let mut s = crate::audio::synth::Synth::new(sr);
        s.apply(AudioCmd::Siren { model, warble });
        s.apply(AudioCmd::Bell(BellPattern::Steady));
        for _ in 0..(skip_s * sr) as usize {
            s.next();
        }
        let v: Vec<f32> = (0..(len_s * sr) as usize).map(|_| s.next()).collect();
        let lo = (sr / (expect * 1.5)) as usize;
        let hi = (sr / (expect * 0.66)) as usize + 1;
        let n = v.len() - hi - 1;
        let corr = |lag: usize| -> f32 { (0..n).map(|i| v[i] * v[i + lag]).sum() };
        let c: Vec<f32> = (lo - 1..=hi + 1).map(corr).collect();
        let mut best = 1;
        for k in 1..c.len() - 1 {
            if c[k] > c[best] {
                best = k;
            }
        }
        let (a, b, d) = (c[best - 1], c[best], c[best + 1]);
        let shift = if a - 2.0 * b + d != 0.0 { 0.5 * (a - d) / (a - 2.0 * b + d) } else { 0.0 };
        sr / ((lo - 1 + best) as f32 + shift)
    };
    let steady = pitch(SirenModel::Wave2F, false, 0.3, 0.5, 2082.0);
    assert!((steady - 2082.0).abs() < 25.0, "WAVE2 steady {}", steady);
    let blaster = pitch(SirenModel::MasterBlaster, false, 2.0, 0.5, 1833.0);
    assert!((blaster - 1833.0).abs() < 40.0, "Master Blaster at full speed {}", blaster);
    let rx = pitch(SirenModel::Rx7c, false, 0.3, 0.5, 3555.0);
    assert!((rx - 3555.0).abs() < 200.0, "RX-7C warble center {}", rx);
}

#[test]
fn empty_room_baseline_stabilizes_door_contact_and_noise_floor() {
    use crate::config::NRect;
    use crate::vision::map::{DoorWatch, RoomBaseline};
    use crate::vision::motion::Motion;

    let (w, h) = (160usize, 120usize);
    let door_rect = NRect::new(0.65, 0.05, 0.30, 0.90);

    // Draw a synthetic white paneled door with frame & handle on the right side.
    let make_room = |door_open: bool, bright_shift: i32| -> Vec<u8> {
        let mut g = vec![145u8; w * h];
        for y in 0..h {
            for x in 0..w {
                let mut v = 145i32;
                if x >= 106 && x <= 150 && y >= 8 && y <= 112 {
                    if door_open {
                        // Dark open hallway instead of the white paneled door
                        v = if x < 120 { 55 } else { 35 + ((y / 8) as i32 * 3) };
                    } else {
                        // White painted door with vertical frame, recessed panels, and dark handle
                        v = 215;
                        if x == 106 || x == 107 || x == 149 || x == 150 {
                            v = 115; // door jamb edge
                        } else if (x >= 114 && x <= 142) && ((y >= 18 && y <= 52) || (y >= 64 && y <= 100)) {
                            v = 192; // paneled recess
                        }
                        if x >= 110 && x <= 116 && y >= 56 && y <= 60 {
                            v = 60; // door handle
                        }
                    }
                }
                g[y * w + x] = (v + bright_shift).clamp(0, 255) as u8;
            }
        }
        g
    };

    let empty = make_room(false, 0);
    let mut std_map = vec![1u8; w * h];
    // Simulate a noisy monitor/bed area on the left (high natural std)
    for y in 60..110 {
        for x in 10..50 {
            std_map[y * w + x] = 14;
        }
    }
    let base = RoomBaseline { w, h, mean: empty.clone(), std: std_map };

    // 1. DoorWatch locked from baseline stays closed under lighting shifts, opens when door opens, closes when shut.
    let mut dw = DoorWatch::new();
    assert!(dw.set_from_baseline(&base, door_rect));
    let t0 = Instant::now();
    let shifted = make_room(false, -28);
    assert_eq!(dw.update(&shifted, w, h, door_rect, false, t0), None);
    assert_eq!(dw.update(&shifted, w, h, door_rect, false, t0 + secs(2)), None);
    assert!(!dw.open, "lighting shift on closed door must NOT flip open (score={})", dw.score);

    let opened = make_room(true, 0);
    let _ = dw.update(&opened, w, h, door_rect, false, t0 + secs(3));
    let ev = dw.update(&opened, w, h, door_rect, false, t0 + secs(4));
    assert_eq!(ev, Some(true), "opening the door must trip DoorWatch (score={})", dw.score);
    assert!(dw.open);

    let _ = dw.update(&shifted, w, h, door_rect, false, t0 + secs(5));
    let ev2 = dw.update(&shifted, w, h, door_rect, false, t0 + secs(6));
    assert_eq!(ev2, Some(false), "closing the door must restore DoorWatch (score={})", dw.score);
    assert!(!dw.open);

    // 2. Motion with calibrated per-pixel noise floor suppresses static jitter in noisy area.
    let mut mot = Motion::new();
    mot.apply_baseline(&base.mean, &base.std, base.w, base.h);
    let mut jitter = empty.clone();
    for y in 65..105 {
        for x in 15..45 {
            jitter[y * w + x] = jitter[y * w + x].saturating_add(24);
        }
    }
    let out = mot.process(&jitter, w, h, 3, Some(door_rect), &[]);
    assert!(out.valid);
    assert!(out.room_frac < 0.005, "calibrated per-pixel noise floor should suppress noisy static area (got {})", out.room_frac);
}

#[test]
fn final_audit_audio_sync_vista_maximum_and_manual_rules() {
    use crate::audio::{synth::Synth, AudioCmd, KeypadVoice};

    // 1. KeyBeep cleanly dips continuous last-10s pulsing entry tone (ShortShortLong) & HiLo alarm,
    //    and cleanly stops in-progress 6160V voice without resuming mid-syllable.
    let mut s = Synth::new(48_000.0);
    s.apply(AudioCmd::Voice(KeypadVoice::Pk5500));
    s.apply(AudioCmd::Sounder(Pattern::ShortShortLong { short_ms: 94, gap_ms: 26, long_ms: 731 }));
    for _ in 0..16_000 {
        s.next();
    }
    s.apply(AudioCmd::KeyBeep);
    for _ in 0..800 {
        s.next();
    }
    let dip_peak = (0..400).map(|_| s.next().abs()).fold(0.0f32, f32::max);
    assert!(dip_peak < 0.02, "KeyBeep must dip cleanly through ShortShortLong entry urgency (got {})", dip_peak);

    let mut sv = Synth::new(48_000.0);
    sv.apply(AudioCmd::Voice(KeypadVoice::Vista6160));
    sv.apply(AudioCmd::Speak(vec!["disarmed".into(), "ready_to_arm".into()]));
    for _ in 0..4_800 {
        sv.next();
    }
    // Pressing a key mid-speech interrupts the phrase cleanly instead of resuming later.
    sv.apply(AudioCmd::KeyBeep);
    for _ in 0..6_000 {
        sv.next();
    }
    let after_key = (0..12_000).map(|_| sv.next().abs()).fold(0.0f32, f32::max);
    assert!(after_key < 0.01, "6160V voice must stay stopped after KeyBeep interruption (got {})", after_key);

    // 2. VISTA-20P MAXIMUM ([Code] + 4): shows ARMED *MAXIMUM* during exit, stays Away + no_entry
    //    when door opens, or Auto-Stays to INSTANT (Stay + no_entry) when door never opens.
    let t0 = Instant::now();
    let mut v = panel(Model::Vista20p);
    keys(&mut v, "12344", t0);
    assert!(v.exit.is_some());
    assert!(v.lcd_vista(t0 + secs(1), 0).lines[0].contains("MAXIMUM"), "LCD should show MAXIMUM during exit");
    v.set_zone(1, true, false, false, t0 + secs(5));
    v.set_zone(1, false, false, false, t0 + secs(8));
    v.tick(t0 + secs(65), 0);
    assert!(matches!(v.armed, Some(a) if a.mode == ArmMode::Away && a.no_entry), "MAXIMUM must arm Away with no_entry");
    // Opening door triggers instant alarm (no entry delay).
    v.set_zone(1, true, false, false, t0 + secs(70));
    v.tick(t0 + secs(70), 0);
    assert!(v.entry.is_none(), "MAXIMUM must have zero entry delay");
    assert!(v.alarm_sounding(t0 + secs(70)));

    // 3. VISTA-20P any-key silences keypad sounder for 10s while external bell stays steady.
    keys(&mut v, "9", t0 + secs(71));
    v.tick(t0 + secs(72), 0);
    assert_eq!(v.sounder_sent, Pattern::Off, "any key mutes 6160V keypad sounder for 10s");
    assert_eq!(v.bell_sent, BellPattern::Steady, "external siren keeps sounding until disarmed");
    v.tick(t0 + secs(82), 0);
    assert!(matches!(v.sounder_sent, Pattern::HiLo { .. }), "keypad sounder resumes after 10s if not disarmed");

    // 4. Disarming Vista emits exactly ONE confirmation tone (no duplicate from Panel::disarm),
    //    and pressing a key cancels pending post-OFF status voice so it never overlaps.
    v.set_zone(1, false, false, false, t0 + secs(83));
    v.out.clear();
    keys(&mut v, "12341", t0 + secs(84)); // 1st OFF silences alarm
    let tones = v.out.iter().filter(|o| matches!(o, Out::Audio(AudioCmd::Tone { .. }))).count();
    assert_eq!(tones, 1, "Vista disarm must emit exactly one confirmation tone (got {})", tones);
    keys(&mut v, "12341", t0 + secs(86)); // 2nd OFF clears alarm memory & schedules vista_pending_status
    assert!(v.vista_pending_status.is_some());
    keys(&mut v, "1", t0 + secs(87)); // new keypress cancels pending status
    assert!(v.vista_pending_status.is_none(), "new keypress must cancel pending status voice");

    // 5. Vista MAXIMUM without opening door converts to INSTANT (Stay + no_entry) via *84.
    let mut v2 = panel(Model::Vista20p);
    keys(&mut v2, "12344", t0);
    v2.tick(t0 + secs(65), 0);
    assert!(matches!(v2.armed, Some(a) if a.mode == ArmMode::Stay && a.no_entry), "MAXIMUM + *84 with no exit becomes INSTANT");
    // Face-disarm on Vista disarms cleanly and schedules status voice.
    v2.set_zone(3, true, false, false, t0 + secs(70)); // instant alarm so face_recognized can disarm
    v2.tick(t0 + secs(70), 0);
    assert!(v2.face_recognized("Tester", t0 + secs(71)));
    assert!(v2.armed.is_none());

    // 6. PC1550 [32] opt 1 suppresses AC trouble, [34] opt 2 squawks bell on exit.
    let mut p15 = panel(Model::Pc1550);
    p15.ee.sections.insert("32".into(), "136".into());
    p15.inputs.ac_ok = false;
    p15.tick(t0 + secs(1), 0);
    assert!(p15.trouble_list().iter().all(|t| !t.contains("AC")), "PC1550 [32] opt 1 suppresses AC trouble");
    p15.ee.sections.insert("34".into(), "24".into());
    p15.out.clear();
    keys(&mut p15, "1234", t0 + secs(2));
    assert!(p15.out.iter().any(|o| matches!(o, Out::Audio(AudioCmd::Squawk(1)))), "PC1550 [34] opt 2 squawks on exit");

    // 7. PC1832 [*][6][Master][1] rejects invalid month/day (99/99) and accepts valid date.
    let mut p18 = panel(Model::Pc1832);
    p18.out.clear();
    keys(&mut p18, "*6123411230999926", t0);
    assert!(
        p18.out.iter().any(|o| matches!(o, Out::Audio(AudioCmd::Beeps { count: 1, on_ms: 2000, .. }))),
        "invalid date 99/99 must produce error tone"
    );
    p18.out.clear();
    keys(&mut p18, "11230112526", t0 + secs(1));
    assert!(
        p18.out.iter().any(|o| matches!(o, Out::Audio(AudioCmd::Beeps { count: 3, .. }))),
        "valid date 11/25/26 must save with 3 confirmation beeps"
    );
}
