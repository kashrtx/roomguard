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
    assert!(matches!(p.sounder_sent, Pattern::Beeps { period_ms: 1000, .. }), "beeps once a second");
    p.tick(t0 + secs(115), 0);
    assert!(matches!(p.sounder_sent, Pattern::Beeps { period_ms: 333, .. }), "three a second at the end");
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
