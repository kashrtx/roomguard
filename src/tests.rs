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
    let mut p = Panel::new(model, PanelSave::fresh(model), labels, "Tester".to_string(), false, Instant::now());
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

#[test]
fn installer_prompt_requires_the_active_panels_master_code() {
    let now = Instant::now();
    for model in [Model::Pc1550, Model::Pc1832, Model::Vista20p] {
        let mut p = panel(model);
        p.ee.set_code(if model == Model::Pc1550 { "2" } else { "04" }, "5678");
        assert!(p.verify_installer_master(model, "5678", now).is_err());
        assert!(p.verify_installer_master(model, model.installer_default(), now).is_err());
        assert!(p.verify_installer_master(model, "1234", now).is_ok());
        p.arm(ArmHow::AwayKey, 0, now);
        assert!(p.verify_installer_master(model, "1234", now).is_err());
    }
    let mut p = panel(Model::Pc1832);
    p.ee.sections.insert("701".into(), "5".into());
    assert!(p.verify_installer_master(Model::Pc1832, "1234", now).is_err());
    assert!(p.verify_installer_master(Model::Pc1832, "123400", now).is_ok());
    p.ee.set_code("40", "654321");
    assert!(p.verify_installer_master(Model::Pc1832, "654321", now).is_ok());
    assert!(p.verify_installer_master(Model::Pc1550, "654321", now).is_err());
}

#[test]
fn webcam_off_stops_the_feed_and_can_restart_the_same_source() {
    use crate::vision::{CamCmd, CamStatus, DEMO_DEVICE};
    setup();
    let (tx, _rx) = std::sync::mpsc::channel();
    let mut cfg = crate::config::CameraCfg { device: DEMO_DEVICE, ..Default::default() };
    let cam = crate::vision::start(cfg.clone(), tx, eframe::egui::Context::default());
    let wait = |powered: bool| {
        let until = Instant::now() + secs(5);
        while Instant::now() < until {
            let matched = {
                let shared = cam.shared.lock().unwrap();
                if powered {
                    shared.status == CamStatus::Running && shared.preview.is_some()
                } else {
                    shared.status == CamStatus::Off && shared.preview.is_none()
                }
            };
            if matched {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("Camera didn't switch to powered={powered}");
    };
    wait(true);
    cfg.enabled = false;
    cam.send(CamCmd::Config(Box::new(cfg.clone())));
    wait(false);
    assert_eq!(cam.shared.lock().unwrap().last_frame_ms, 0);
    cfg.enabled = true;
    cam.send(CamCmd::Config(Box::new(cfg)));
    wait(true);
    cam.send(CamCmd::Shutdown);
    cam.wait_stopped(secs(3));
}

#[test]
fn all_panels_follow_physical_arming_and_keep_guarding_when_the_link_drops() {
    use crate::home_assistant::AlarmState::*;

    for model in [Model::Pc1550, Model::Pc1832, Model::Vista20p] {
        let t = Instant::now();
        let mut p = panel(model);
        p.set_following_panel(true, t);
        p.sync_physical_panel(Some(Disarmed), false, t);
        keys(&mut p, "12342", t);
        assert!(!p.is_on(), "{model:?} accepted a virtual arming code");

        p.sync_physical_panel(Some(Arming), true, t);
        assert!(p.is_on());
        assert_eq!(p.status(t).0, StatusKind::Exit);
        p.tick(t + secs(300), 0);
        assert!(p.armed.is_none() && p.exit.is_none(), "{model:?} invented an exit countdown");

        p.sync_physical_panel(Some(Stay), true, t + secs(301));
        assert_eq!(p.armed.unwrap().mode, ArmMode::Stay);
        for zone in 1..=p.n() {
            assert_eq!(p.zones[zone - 1].auto_bypass, p.cfg(zone).kind.is_stay_away());
        }
        p.sync_physical_panel(Some(Night), true, t + secs(302));
        assert_eq!(p.armed.unwrap().mode, ArmMode::Stay);
        assert_eq!(p.vista_night, model == Model::Vista20p);

        p.sync_physical_panel(Some(Away), true, t + secs(303));
        assert_eq!(p.armed.unwrap().mode, ArmMode::Away);
        assert!(p.zones.iter().all(|z| !z.auto_bypass));
        p.sync_physical_panel(None, true, t + secs(304));
        p.tick(t + secs(314), 0);
        assert_eq!(p.armed.unwrap().mode, ArmMode::Away);
        assert!(p.physical_unavailable_since.is_some());
        assert_ne!(p.troubles & crate::engine::panel::tb::HOME_PANEL, 0);
        keys(&mut p, "12341", t + secs(315));
        assert!(!p.face_recognized("Tester", t + secs(315)));
        assert_eq!(p.armed.unwrap().mode, ArmMode::Away);

        let mut saved = p.to_save(true);
        if model == Model::Vista20p {
            saved.ee.sections.insert("39".into(), "0".into());
        }
        let mut p = Panel::new(model, saved, crate::config::default_labels(model), "Tester".into(), true, t + secs(316));
        assert_eq!(p.armed.unwrap().mode, ArmMode::Away);
        p.sync_physical_panel(Some(Away), false, t + secs(317));
        p.sync_physical_panel(Some(Disarmed), true, t + secs(318));
        assert!(!p.is_on());
        assert!(p.out.iter().any(|out| matches!(out, Out::Armed(false))));
    }
}

#[test]
fn physical_entry_and_alarm_follow_bridge_timing_and_alert_once() {
    use crate::home_assistant::AlarmState::*;

    for model in [Model::Pc1550, Model::Pc1832, Model::Vista20p] {
        let t = Instant::now();
        let mut p = panel(model);
        p.set_following_panel(true, t);
        p.sync_physical_panel(Some(Away), false, t);
        p.sync_physical_panel(Some(Pending), true, t + secs(1));
        assert_eq!(p.status(t + secs(1)).0, StatusKind::Entry);
        assert!(p.entry.is_none());
        p.tick(t + secs(300), 0);
        assert!(p.alarms.is_empty());
        p.sync_physical_panel(Some(Triggered), true, t + secs(301));
        p.sync_physical_panel(None, false, t + secs(302));
        p.sync_physical_panel(Some(Triggered), true, t + secs(303));
        let at = t + secs(304);
        assert_eq!(p.status(at).0, StatusKind::Alarm);
        p.tick(at, 0);
        assert_eq!(p.bell_sent, BellPattern::Off);
        let reports = |kind| p.out.iter().filter(|out| matches!(out, Out::PhoneReport(report) if report.kind == kind)).count();
        assert_eq!(reports(ReportKind::PhysicalEntry), 1);
        assert_eq!(reports(ReportKind::PhysicalAlarm), 1);
        p.sync_physical_panel(Some(Disarmed), true, at);
        assert!(!p.is_on());
        p.sync_physical_panel(Some(Triggered), true, at + secs(1));
        assert!(p.armed.is_none(), "an alarm while disarmed is not evidence of arming");
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
    let p2 = Panel::new(Model::Pc1832, back, crate::config::default_labels(Model::Pc1832), "Tester".to_string(), false, Instant::now());
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
    keys(&mut v, "9999", t0 + secs(79));
    v.tick(t0 + secs(80), 0);
    assert_eq!(v.sounder_sent, Pattern::Off);
    v.tick(t0 + secs(82), 0);
    assert!(matches!(v.sounder_sent, Pattern::HiLo { .. }), "keypad sounder resumes 10s after the first key, even after more digits");

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

// ----- RoomGuard 2.2 -----

#[test]
fn stay_mode_does_not_show_manual_bypass_on_vista_or_zone_view() {
    let t0 = Instant::now();
    // 1. Vista20p Stay arming auto-bypasses interior zones silently without displaying "ZONE BYPASSED".
    let mut v = panel(Model::Vista20p);
    keys(&mut v, "12343", t0);
    v.tick(t0 + secs(65), 0);
    assert!(matches!(v.armed, Some(a) if a.mode == ArmMode::Stay));
    assert!(v.zones[1].bypassed && v.zones[1].auto_bypass, "zone 2 should be auto-bypassed in Stay");
    assert!(!v.manual_bypass_any(), "auto-bypass must not count as a manual bypass");
    let lcd = v.lcd_vista(t0 + secs(66), 0);
    assert!(lcd.lines[1].contains("ALL SECURE"), "expected ALL SECURE on Stay, got {:?}", lcd.lines);
    assert!(!lcd.lines[1].contains("BYPASS"), "Stay must not falsely show ZONE BYPASSED: {:?}", lcd.lines);
    let zv = v.zone_views();
    assert!(zv[1].bypassed && zv[1].auto_bypass);

    // 2. Explicit user bypass ([Code] + 6 + 03) DOES count as manual bypass and displays DISARMED BYPASS / ZONE BYPASSED.
    keys(&mut v, "12341", t0 + secs(70));
    keys(&mut v, "1234603", t0 + secs(72));
    assert!(v.zones[2].bypassed && !v.zones[2].auto_bypass);
    assert!(v.manual_bypass_any());
    let lcd_confirm = v.lcd_vista(t0 + secs(73), 0);
    assert!(lcd_confirm.lines[0].contains("BYPAS 03"), "expected bypass confirmation, got {:?}", lcd_confirm.lines);
    let lcd_dis = v.lcd_vista(t0 + secs(76), 0);
    assert!(lcd_dis.lines[0].contains("BYPASS"), "manual bypass while disarmed must show DISARMED BYPASS: {:?}", lcd_dis.lines);
    keys(&mut v, "12343", t0 + secs(77));
    v.tick(t0 + secs(142), 0);
    let lcd_arm = v.lcd_vista(t0 + secs(143), 0);
    assert!(lcd_arm.lines[1].contains("BYPASS"), "manual bypass while armed Stay must show ZONE BYPASSED: {:?}", lcd_arm.lines);
    let zv2 = v.zone_views();
    assert!(zv2[2].bypassed && !zv2[2].auto_bypass);

    // 3. PC1832 Auto-Stay also marks zone 2 as auto_bypass and not manual_bypass_any.
    let mut p18 = panel(Model::Pc1832);
    keys(&mut p18, "1234", t0);
    p18.tick(t0 + secs(125), 0);
    assert!(matches!(p18.armed, Some(a) if a.mode == ArmMode::Stay));
    assert!(p18.zones[1].bypassed && p18.zones[1].auto_bypass);
    assert!(!p18.manual_bypass_any());
}

#[test]
fn vista_exit_error_and_camera_fault_do_not_fire_instant_burglary() {
    let t0 = Instant::now();
    // 1. Vista20p Away with interior Zone 2 still open at exit expiry starts Entry Delay (Exit Error),
    //    not an instant zero-delay burglary alarm.
    let mut v = panel(Model::Vista20p);
    keys(&mut v, "12342", t0);
    v.set_zone(1, true, false, false, t0 + secs(5));
    v.set_zone(1, false, false, false, t0 + secs(8));
    v.set_zone(2, true, false, false, t0 + secs(58));
    v.tick(t0 + secs(65), 0);
    assert!(matches!(v.armed, Some(a) if a.mode == ArmMode::Away));
    assert!(v.entry.is_some(), "open Stay/Away zone at Vista exit expiry should start Entry Delay");
    assert!(v.alarms.is_empty(), "Vista Exit Error must not fire an instant zero-delay alarm");

    // 2. Pure camera fault (disconnect/frame drop) on an armed PC1832 does NOT trigger an instant siren.
    let mut p = panel(Model::Pc1832);
    armed_away(&mut p, t0);
    p.set_zone(1, false, false, true, t0 + secs(20));
    p.tick(t0 + secs(21), 0);
    assert!(p.alarms.is_empty(), "camera fault without open/tamper must not trigger an instant alarm");
    assert_eq!(p.bell_sent, BellPattern::Off);

    // 3. Zone tamper while armed triggers a single Tamper alarm without duplicating into Burglary.
    p.set_zone(1, false, true, false, t0 + secs(25));
    p.tick(t0 + secs(26), 0);
    assert_eq!(p.alarms.len(), 1, "tamper should raise exactly one alarm, got {:?}", p.alarms);
    assert_eq!(p.alarms[0].kind, AlarmKind::Tamper);
}

#[test]
fn camera_multi_zone_config_motion_and_contacts_work() {
    use crate::config::{CameraCfg, NRect};
    use crate::vision::motion::Motion;

    let mut cfg = CameraCfg::default();
    let r1 = NRect::new(0.05, 0.10, 0.25, 0.80);
    let r3 = NRect::new(0.40, 0.15, 0.22, 0.45);
    let r5 = NRect::new(0.72, 0.20, 0.22, 0.50);

    cfg.set_zone_rect(1, r1, true);
    cfg.set_zone_rect(3, r3, true);
    cfg.set_zone_rect(5, r5, false);

    assert!(cfg.uses_room_fallback(), "with Z1 drawn and Z2 not explicitly drawn, Z2 covers the room");
    assert_eq!(cfg.camera_mask(), [true, true, true, false, true, false, false, false]);
    assert!(cfg.zone_contact(1));
    assert!(cfg.zone_contact(3));
    assert!(!cfg.zone_contact(5));

    // Drawing Z2 explicitly replaces whole-room fallback.
    let r2 = NRect::new(0.32, 0.55, 0.30, 0.35);
    cfg.set_zone_rect(2, r2, false);
    assert!(!cfg.uses_room_fallback());
    assert_eq!(cfg.zone_rect(2), Some(r2));

    // Toggle contact and remove zone.
    cfg.set_zone_contact(3, false);
    assert!(!cfg.zone_contact(3));
    cfg.remove_zone(3);
    assert_eq!(cfg.zone_rect(3), None);
    assert_eq!(cfg.camera_mask(), [true, true, false, false, true, false, false, false]);

    // Round-trip & backward compatibility with older JSON lacking `zones`.
    let json = serde_json::to_string(&cfg).expect("serializes");
    let back: CameraCfg = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(cfg, back);
    let legacy: CameraCfg = serde_json::from_str(r#"{"enabled":true,"doorway":{"x":0.1,"y":0.2,"w":0.2,"h":0.5}}"#).expect("legacy loads");
    assert!(legacy.zones.is_empty());
    assert!(legacy.uses_room_fallback());

    // Motion::rect_frac isolates per-zone motion cleanly.
    let (w, h) = (160usize, 120usize);
    let mut mot = Motion::new();
    let empty = vec![120u8; w * h];
    mot.process(&empty, w, h, 3, Some(r1), &[]);
    let mut moved = empty.clone();
    // Draw a bright moving patch strictly inside r3 (x: 0.42..0.60, y: 0.20..0.55).
    for y in 24..66 {
        for x in 68..96 {
            moved[y * w + x] = 225;
        }
    }
    let out = mot.process(&moved, w, h, 3, Some(r1), &[]);
    assert!(out.valid);
    assert!(mot.rect_frac(r3) > 0.25, "expected motion inside Z3, got {}", mot.rect_frac(r3));
    assert!(mot.rect_frac(r5) < 0.01, "Z5 should stay quiet, got {}", mot.rect_frac(r5));
}

#[test]
fn icon_assets_have_valid_windows_dib_and_macos_chunks() {
    // 1. Windows ICO: 7 frames, standard 32-bit DIB for 16..128 and PNG for 256.
    let ico = include_bytes!("../assets/icon/icon.ico");
    assert!(ico.len() > 64);
    assert_eq!(u16::from_le_bytes([ico[0], ico[1]]), 0);
    assert_eq!(u16::from_le_bytes([ico[2], ico[3]]), 1);
    let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
    assert_eq!(count, 7);
    let mut saw_png_256 = false;
    let mut dib_count = 0;
    for i in 0..count {
        let e = 6 + i * 16;
        let w = ico[e];
        let h = ico[e + 1];
        let sz = u32::from_le_bytes([ico[e + 8], ico[e + 9], ico[e + 10], ico[e + 11]]) as usize;
        let off = u32::from_le_bytes([ico[e + 12], ico[e + 13], ico[e + 14], ico[e + 15]]) as usize;
        let frame = &ico[off..off + sz];
        if w == 0 && h == 0 {
            assert!(frame.starts_with(b"\x89PNG\r\n\x1a\n"), "256px ICO entry must be PNG");
            saw_png_256 = true;
        } else {
            assert_eq!(u32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]), 40, "ICO {}px entry must be 40-byte BITMAPINFOHEADER DIB", w);
            dib_count += 1;
        }
    }
    assert!(saw_png_256 && dib_count == 6);

    // 2. macOS ICNS: valid header and complete chunk set (16px through 1024px / Retina).
    let icns = include_bytes!("../assets/icon/icon.icns");
    assert_eq!(&icns[0..4], b"icns");
    let total = u32::from_be_bytes([icns[4], icns[5], icns[6], icns[7]]) as usize;
    assert_eq!(total, icns.len());
    let mut chunks = Vec::new();
    let mut pos = 8usize;
    while pos + 8 <= icns.len() {
        chunks.push([icns[pos], icns[pos + 1], icns[pos + 2], icns[pos + 3]]);
        let clen = u32::from_be_bytes([icns[pos + 4], icns[pos + 5], icns[pos + 6], icns[pos + 7]]) as usize;
        assert!(clen >= 8 && pos + clen <= icns.len());
        pos += clen;
    }
    for want in [b"icp4", b"icp5", b"icp6", b"ic07", b"ic08", b"ic09", b"ic10", b"ic11", b"ic12", b"ic13", b"ic14"] {
        assert!(chunks.contains(want), "missing macOS ICNS chunk {:?}", std::str::from_utf8(want));
    }
}

#[test]
fn vista_stay_and_instant_arm_with_the_low_tu_tu_tu() {
    let t0 = Instant::now();
    let tu_tu_tu = |p: &Panel| {
        p.out.iter().any(|o| matches!(o, Out::Audio(AudioCmd::ToneSeq { tones, .. }) if tones.len() == 3 && tones.iter().all(|t| t.0 == 1019)))
    };
    for code in ["12343", "12347"] {
        let mut p = panel(Model::Vista20p);
        p.out.clear();
        keys(&mut p, code, t0);
        assert!(p.exit.is_some(), "{} arms", code);
        assert!(tu_tu_tu(&p), "{} gives the low tu-tu-tu", code);
        p.tick(t0 + secs(2), 0);
        assert_eq!(p.sounder_sent, Pattern::Off, "{} counts down quietly", code);
    }
    // AWAY gets the exit beeps instead, with no tu-tu-tu.
    let mut p = panel(Model::Vista20p);
    p.out.clear();
    keys(&mut p, "12342", t0);
    assert!(p.exit.is_some());
    assert!(!tu_tu_tu(&p), "AWAY has no tu-tu-tu");
    p.tick(t0 + secs(2), 0);
    assert!(matches!(p.sounder_sent, Pattern::Beeps { .. }), "AWAY beeps through its exit delay");
}

#[test]
fn exit_expiry_supervisory_fault_does_not_false_alarm() {
    let t0 = Instant::now();
    // Brief camera disconnect (fault only, not open or tamper) at exit expiry
    // must NOT start an audible exit-fault / burglary alarm.
    let mut p = panel(Model::Vista20p);
    keys(&mut p, "12342", t0);
    assert!(p.exit.is_some());
    p.set_zone(1, false, false, true, t0 + secs(5));
    p.tick(t0 + secs(90), 0);
    assert!(p.armed.is_some(), "panel should finish arming");
    assert!(p.entry.is_none(), "supervisory fault alone must not trigger exit-fault entry delay");
    assert!(p.alarms.is_empty(), "supervisory fault alone must not trigger burglary alarm");

    // By contrast, a zone tamper at exit expiry must trigger a Tamper alarm immediately.
    let mut p2 = panel(Model::Vista20p);
    keys(&mut p2, "12342", t0);
    p2.set_zone(1, false, true, false, t0 + secs(5));
    p2.tick(t0 + secs(90), 0);
    assert!(p2.alarms.iter().any(|a| a.kind == AlarmKind::Tamper), "tamper at exit expiry must trigger Tamper");
}

#[test]
fn tone_seq_waits_before_voice_and_drains_interrupt_fade() {
    use crate::audio::synth::Synth;
    use crate::audio::KeypadVoice;
    let sr = 48_000.0;
    let mut s = Synth::new(sr);
    s.apply(AudioCmd::Voice(KeypadVoice::Vista6160));
    s.apply(AudioCmd::Volumes { master: 1.0, siren: 0.0, keypad: 1.0, line: 0.0 });
    // Start speaking a real word and advance partway into voiced speech.
    s.apply(AudioCmd::Speak(vec!["armed".into(), "stay".into()]));
    let mut last = 0.0f32;
    for _ in 0..8000 {
        last = s.next();
    }
    // Interrupt mid-speech with a keypress + ToneSeq + new Speak (NIGHT-STAY).
    s.apply(AudioCmd::KeyBeep);
    s.apply(AudioCmd::ToneSeq { delay_ms: 550, tones: vec![(1019, 150, 50), (1019, 175, 75), (1019, 240, 500)] });
    s.apply(AudioCmd::Speak(vec!["armed".into(), "stay".into()]));
    // First 6 ms must drain the voice stop-fade immediately without a hard step.
    let first_sample = s.next();
    assert!((first_sample - last).abs() < 0.35, "unexpected hard click on voice interrupt: {} -> {}", last, first_sample);
    // Advance past the 46+22 ms key beep + 550 ms delay + tu-tu-tu + 500 ms tail (~1.85 s),
    // verifying voice actually speaks after ToneSeq completes.
    for _ in 0..(sr as usize * 185 / 100) {
        let _ = s.next();
    }
    let mut speech_peak = 0.0f32;
    for _ in 0..(sr as usize / 2) {
        speech_peak = speech_peak.max(s.next().abs());
    }
    assert!(speech_peak > 0.05, "expected spoken phrase after ToneSeq, peak={}", speech_peak);
}

#[test]
fn voice_rom_equalizes_all_76_words_to_uniform_perceived_loudness() {
    let rows = crate::audio::voice::audit_equalized_loudness();
    assert_eq!(rows.len(), 76, "expected all 76 voice ROM words");
    let armed = rows.iter().find(|r| r.0 == "armed").expect("armed token").2;
    let stay = rows.iter().find(|r| r.0 == "stay").expect("stay token").2;
    let away = rows.iter().find(|r| r.0 == "away").expect("away token").2;
    let instant = rows.iter().find(|r| r.0 == "instant").expect("instant token");
    let exit_now = rows.iter().find(|r| r.0 == "exit_now").expect("exit_now token").2;
    assert!((armed - stay).abs() / stay < 0.03, "armed ({}) should match stay ({})", armed, stay);
    assert!((armed - away).abs() / away < 0.03, "armed ({}) should match away ({})", armed, away);
    assert!((armed - instant.2).abs() / instant.2 < 0.03, "armed ({}) should match instant ({})", armed, instant.2);
    assert!(instant.1 < 0.85, "instant sharp syllable must be attenuated (gain={})", instant.1);
    assert!((armed - exit_now).abs() / exit_now < 0.03, "armed ({}) should match exit_now ({})", armed, exit_now);
    let min_l = rows.iter().map(|r| r.2).fold(f32::MAX, f32::min);
    let max_l = rows.iter().map(|r| r.2).fold(0.0f32, f32::max);
    let spread_db = 20.0 * (max_l / min_l.max(1e-6)).log10();
    assert!(spread_db < 0.6, "all 76 equalized words must be within 0.6 dB spread (got {:.2} dB)", spread_db);
}

#[test]
fn keypress_ducks_exit_and_entry_countdown_beeps_without_overlap() {
    use crate::audio::synth::Synth;
    use crate::audio::KeypadVoice;
    let sr = 48_000.0;
    let mut s = Synth::new(sr);
    s.apply(AudioCmd::Voice(KeypadVoice::Vista6160));
    s.apply(AudioCmd::Volumes { master: 1.0, siren: 0.0, keypad: 1.0, line: 0.0 });
    // Start active exit countdown beeps (500ms on / 510ms off).
    s.apply(AudioCmd::Sounder(Pattern::Beeps { on_ms: 500, off_ms: 0, count: 1, period_ms: 1010 }));
    for _ in 0..4800 {
        let _ = s.next();
    }
    // User presses first digit of disarm code during countdown:
    s.apply(AudioCmd::KeyBeep);
    // Advance past the 46ms + 22ms key chirp (~100ms).
    for _ in 0..4800 {
        let _ = s.next();
    }
    // For the next 1.2 seconds while typing the remaining digits, exit beeps must stay quiet!
    let mut ducked_peak = 0.0f32;
    for _ in 0..(sr as usize * 12 / 10) {
        ducked_peak = ducked_peak.max(s.next().abs());
    }
    assert!(ducked_peak < 0.02, "exit countdown must stay quiet while entering code (peak={})", ducked_peak);
    // If the user stops typing without disarming, exit beeps resume after the ~1.8s duck window.
    for _ in 0..(sr as usize) {
        let _ = s.next();
    }
    let mut resumed_peak = 0.0f32;
    for _ in 0..(sr as usize * 12 / 10) {
        resumed_peak = resumed_peak.max(s.next().abs());
    }
    assert!(resumed_peak > 0.05, "exit countdown should resume if user stops typing (peak={})", resumed_peak);
}

#[test]
fn more_keypresses_do_not_extend_the_running_sounder_pause() {
    use crate::audio::synth::Synth;
    for voice in [crate::audio::KeypadVoice::Classic, crate::audio::KeypadVoice::Pk5500, crate::audio::KeypadVoice::Vista6160] {
        let mut s = Synth::new(48_000.0);
        s.apply(AudioCmd::Voice(voice));
        s.apply(AudioCmd::Sounder(Pattern::Steady));
        for _ in 0..4 {
            s.apply(AudioCmd::KeyBeep);
            for _ in 0..19_200 {
                s.next();
            }
        }
        let quiet = (0..4_800).map(|_| s.next().abs()).fold(0.0, f32::max);
        assert!(quiet < 0.02);
        for _ in 0..4_800 {
            s.next();
        }
        let resumed = (0..4_800).map(|_| s.next().abs()).fold(0.0, f32::max);
        assert!(resumed > 0.05, "{voice:?} should resume 1.8s after the first key");
    }
}

#[test]
fn vista_announces_all_arming_modes_at_start_and_completion() {
    let speaks = |p: &Panel| -> Vec<Vec<String>> {
        p.out
            .iter()
            .filter_map(|o| match o {
                Out::Audio(AudioCmd::Speak(toks)) => Some(toks.clone()),
                _ => None,
            })
            .collect()
    };
    let t0 = Instant::now();

    // 1. AWAY with door exit: "Armed away, exit now" during exit -> "Armed away" at t=0.
    let mut p = panel(Model::Vista20p);
    p.out.clear();
    keys(&mut p, "12342", t0);
    p.tick(t0 + secs(3), 0);
    assert!(speaks(&p).contains(&vec!["armed".into(), "away".into(), "exit_now".into()]), "AWAY exit voice");
    p.set_zone(1, true, false, false, t0 + secs(5));
    p.set_zone(1, false, false, false, t0 + secs(8));
    p.out.clear();
    p.tick(t0 + secs(65), 0);
    assert_eq!(p.armed.map(|a| a.mode), Some(ArmMode::Away));
    assert!(speaks(&p).contains(&vec!["armed".into(), "away".into()]), "AWAY completion voice");

    // 2. AWAY without door exit (*84 Auto-Stay): finishes as STAY and announces "Armed stay".
    let mut p2 = panel(Model::Vista20p);
    keys(&mut p2, "12342", t0);
    p2.tick(t0 + secs(3), 0);
    p2.out.clear();
    p2.tick(t0 + secs(65), 0);
    assert_eq!(p2.armed.map(|a| a.mode), Some(ArmMode::Stay));
    assert!(speaks(&p2).contains(&vec!["armed".into(), "stay".into()]), "Auto-Stay completion voice");

    // 3. STAY: "Armed stay" at start (no exit_now) -> "Armed stay" at completion.
    let mut p3 = panel(Model::Vista20p);
    p3.out.clear();
    keys(&mut p3, "12343", t0);
    assert!(speaks(&p3).contains(&vec!["armed".into(), "stay".into()]), "STAY start voice");
    assert!(!speaks(&p3).iter().any(|v| v.contains(&"exit_now".to_string())), "STAY must not say exit_now");
    p3.out.clear();
    p3.tick(t0 + secs(65), 0);
    assert!(speaks(&p3).contains(&vec!["armed".into(), "stay".into()]), "STAY completion voice");

    // 4. NIGHT-STAY: "Armed night stay" at start AND at completion AND on # + 0 + * status.
    let mut p4 = panel(Model::Vista20p);
    p4.out.clear();
    keys(&mut p4, "123433", t0);
    assert!(speaks(&p4).contains(&vec!["armed".into(), "night".into(), "stay".into()]), "NIGHT-STAY start voice");
    p4.out.clear();
    p4.tick(t0 + secs(65), 0);
    assert!(speaks(&p4).contains(&vec!["armed".into(), "night".into(), "stay".into()]), "NIGHT-STAY completion voice");
    p4.out.clear();
    keys(&mut p4, "#0*", t0 + secs(70));
    assert!(speaks(&p4).contains(&vec!["armed".into(), "night".into(), "stay".into()]), "NIGHT-STAY status voice");

    // 5. INSTANT: "Armed stay instant" at start (no exit_now) -> "Armed stay instant" at completion.
    let mut p5 = panel(Model::Vista20p);
    p5.out.clear();
    keys(&mut p5, "12347", t0);
    assert!(speaks(&p5).contains(&vec!["armed".into(), "stay".into(), "instant".into()]), "INSTANT start voice");
    assert!(!speaks(&p5).iter().any(|v| v.contains(&"exit_now".to_string())), "INSTANT must not say exit_now");
    p5.out.clear();
    p5.tick(t0 + secs(65), 0);
    assert!(speaks(&p5).contains(&vec!["armed".into(), "stay".into(), "instant".into()]), "INSTANT completion voice");

    // 6. MAXIMUM with door exit: "Armed maximum, exit now" during exit -> "Armed maximum" at completion.
    let mut p6 = panel(Model::Vista20p);
    p6.out.clear();
    keys(&mut p6, "12344", t0);
    p6.tick(t0 + secs(3), 0);
    assert!(speaks(&p6).contains(&vec!["armed".into(), "maximum".into(), "exit_now".into()]), "MAXIMUM exit voice");
    p6.set_zone(1, true, false, false, t0 + secs(5));
    p6.set_zone(1, false, false, false, t0 + secs(8));
    p6.out.clear();
    p6.tick(t0 + secs(65), 0);
    assert!(speaks(&p6).contains(&vec!["armed".into(), "maximum".into()]), "MAXIMUM completion voice");
}

#[test]
fn vista_6160v_bypass_voice_is_silent_on_entry_and_speaks_on_status_and_arming() {
    let speaks = |p: &Panel| -> Vec<Vec<String>> {
        p.out
            .iter()
            .filter_map(|o| match o {
                Out::Audio(AudioCmd::Speak(toks)) => Some(toks.clone()),
                _ => None,
            })
            .collect()
    };
    let t0 = Instant::now();
    let mut p = panel(Model::Vista20p);
    p.chime = true;

    // 1. Bypassing zone 03 emits confirmation beep ONLY, NO spoken words!
    p.out.clear();
    keys(&mut p, "1234603", t0);
    assert!(p.zones[2].bypassed && !p.zones[2].auto_bypass);
    assert!(speaks(&p).is_empty(), "6160V must NOT speak on bypass entry, got {:?}", speaks(&p));

    // 2. Opening a bypassed zone with Voice Chime ON stays completely silent!
    p.out.clear();
    p.set_zone(3, true, false, false, t0 + secs(2));
    assert!(speaks(&p).is_empty(), "opening bypassed zone must stay silent even with Voice Chime ON");
    p.set_zone(3, false, false, false, t0 + secs(3));

    // 3. Pressing Status (# + 0 + *) announces "Disarmed, bypassed, ready to arm" without zone names.
    p.out.clear();
    keys(&mut p, "#0*", t0 + secs(5));
    assert_eq!(
        speaks(&p),
        vec![vec!["disarmed".to_string(), "bypassed".to_string(), "ready_to_arm".to_string()]],
        "disarmed status with manual bypass"
    );

    // 4. Arming STAY with manual bypass speaks "Armed stay, zone bypassed" at start, completion, and status.
    p.out.clear();
    keys(&mut p, "12343", t0 + secs(10));
    assert!(speaks(&p).contains(&vec!["armed".into(), "stay".into(), "zone".into(), "bypassed".into()]));
    p.out.clear();
    p.tick(t0 + secs(75), 0);
    assert!(speaks(&p).contains(&vec!["armed".into(), "stay".into(), "zone".into(), "bypassed".into()]));
    p.out.clear();
    keys(&mut p, "#0*", t0 + secs(80));
    assert!(speaks(&p).contains(&vec!["armed".into(), "stay".into(), "zone".into(), "bypassed".into()]));

    // 5. Disarm (removes bypass), re-arm STAY cleanly: auto-bypassed interior does NOT speak "zone bypassed"!
    keys(&mut p, "12341", t0 + secs(85));
    assert!(!p.manual_bypass_any());
    p.out.clear();
    keys(&mut p, "12343", t0 + secs(90));
    p.tick(t0 + secs(155), 0);
    assert!(!speaks(&p).iter().any(|v| v.contains(&"bypassed".to_string())), "Stay auto-bypass must not speak bypassed");
}

#[test]
fn vista_time_date_63_event_log_60_and_function_key_02() {
    let t0 = Instant::now();
    let wall = crate::util::now_ms();
    let mut p = panel(Model::Vista20p);
    p.tick(t0, wall);

    // 1. [Master] + # + 63 opens Time/Date display; [*] edits; typing 10:12PM 07/11/19 (THU) + [*] saves & logs CID 625!
    keys(&mut p, "1234#63", t0 + secs(1));
    let view1 = p.lcd_vista(t0 + secs(1), wall);
    assert!(view1.lines[0].starts_with("TIME/DATE"), "expected TIME/DATE view, got {:?}", view1.lines);
    assert!(view1.cursor.is_none());

    keys(&mut p, "*", t0 + secs(2));
    let edit_start = p.lcd_vista(t0 + secs(2), wall);
    assert_eq!(edit_start.cursor, Some((1, 0)), "cursor starts under 1st hour digit");

    // 10:12 PM (1=PM), year 19, month 07, day 11 -> Thursday, July 11, 2019 10:12 PM
    keys(&mut p, "10121190711*", t0 + secs(3));
    assert!(p.events.iter().any(|e| e.cid == 625), "setting clock must log CID 625");

    // Verify #63 now displays the exact manual example: TIME/DATE   THU / 10:12PM 07/11/19!
    keys(&mut p, "1234#63", t0 + secs(5));
    let view2 = p.lcd_vista(t0 + secs(5), wall);
    assert_eq!(view2.lines[0].trim_end(), "TIME/DATE   THU");
    assert_eq!(view2.lines[1].trim_end(), "10:12PM 07/11/19");
    // 30s inactivity timeout exits cleanly back to normal
    p.tick(t0 + secs(36), wall);
    assert!(p.lcd_vista(t0 + secs(36), wall).lines[0].contains("DISARMED"));

    // 2. Program *57 Key D = 02 (Display Time/Date) -> holding Key D shows TIME/DATE without a code!
    p.ee.sections.insert("57".into(), "00000002".into());
    p.key(Key::Away, true, t0 + secs(40));
    p.tick(t0 + secs(42) + Duration::from_millis(100), wall);
    p.key(Key::Away, false, t0 + secs(42) + Duration::from_millis(200));
    let fn_view = p.lcd_vista(t0 + secs(43), wall);
    assert_eq!(fn_view.lines[0].trim_end(), "TIME/DATE   THU");
    assert_eq!(fn_view.lines[1].trim_end(), "10:12PM 07/11/19");
    // Function key view is view-only: pressing * exits rather than editing
    keys(&mut p, "*", t0 + secs(44));
    assert!(p.lcd_vista(t0 + secs(44), wall).lines[0].contains("DISARMED"));

    // 3. [Master] + # + 60 opens 100-Event History Log in authentic Contact ID format; * = older, # = newer.
    keys(&mut p, "1234603", t0 + secs(50)); // logs CID 570 bypass
    keys(&mut p, "1234#60", t0 + secs(52));
    let ev1 = p.lcd_vista(t0 + secs(52), wall);
    assert!(ev1.lines[0].starts_with("001 E570 Z003 P1"), "expected bypass CID 570 first, got {:?}", ev1.lines);
    assert_eq!(ev1.lines[1].trim_end(), "10:12PM 07/11/19");
    keys(&mut p, "*", t0 + secs(53)); // older -> CID 625 clock change
    let ev2 = p.lcd_vista(t0 + secs(53), wall);
    assert!(ev2.lines[0].starts_with("002 E625 U002 P1"), "expected older CID 625 event, got {:?}", ev2.lines);
    keys(&mut p, "#", t0 + secs(54)); // newer -> back to 001
    assert!(p.lcd_vista(t0 + secs(54), wall).lines[0].starts_with("001 "));
    keys(&mut p, "1", t0 + secs(55)); // any other key exits
    assert!(p.lcd_vista(t0 + secs(55), wall).lines[0].contains("DISARMED"));

    // *90 Event Log Enables filter: set *90 = 01 (alarms only) -> empty when no alarms logged!
    p.ee.sections.insert("90".into(), "01".into());
    assert!(p.vista_event_log_entries().is_empty());
}

#[test]
fn vista_user_attributes_authority_levels_and_installer_master_code() {
    let t0 = Instant::now();
    let mut p = panel(Model::Vista20p);

    // 1. Installer (4112) + 8 + 02 + 2468 sets Master code in a SINGLE entry (no double entry)!
    keys(&mut p, "41128022468", t0);
    assert!(p.ee.code_matches("02", "2468"), "installer must change master code in one entry");

    // 2. Master adds User 03 (Arm-Only = auth 1), User 04 (Guest = auth 2), User 05 (Duress = auth 3)
    keys(&mut p, "24688033333", t0 + secs(2));
    keys(&mut p, "2468803#11", t0 + secs(4));
    keys(&mut p, "24688044444", t0 + secs(6));
    keys(&mut p, "2468804#12", t0 + secs(8));
    keys(&mut p, "24688055555", t0 + secs(10));
    keys(&mut p, "2468805#13", t0 + secs(12));
    assert_eq!(p.ee.attrs("03") & 0x07, 1);
    assert_eq!(p.ee.attrs("04") & 0x07, 2);
    assert_eq!(p.ee.attrs("05") & 0x07, 3);

    // Arm-Only (03) can arm STAY, but CANNOT disarm!
    keys(&mut p, "33333", t0 + secs(15));
    assert!(p.exit.is_some(), "Arm-Only user can arm");
    keys(&mut p, "33331", t0 + secs(18));
    assert!(p.exit.is_some(), "Arm-Only user must NOT disarm");

    // Guest (04) CANNOT disarm a system armed by User 03!
    keys(&mut p, "44441", t0 + secs(20));
    assert!(p.exit.is_some(), "Guest must NOT disarm when another user armed");

    // Master disarms cleanly; Guest arms and CAN disarm its own arming!
    keys(&mut p, "24681", t0 + secs(22));
    assert!(p.exit.is_none() && p.armed.is_none());
    keys(&mut p, "44443", t0 + secs(25));
    assert!(p.exit.is_some());
    keys(&mut p, "44441", t0 + secs(28));
    assert!(p.exit.is_none() && p.armed.is_none(), "Guest MUST disarm when Guest itself armed");

    // Duress (05) disarms silently AND sends CID 121 Duress report!
    keys(&mut p, "24683", t0 + secs(32));
    p.out.clear();
    keys(&mut p, "55551", t0 + secs(35));
    assert!(p.exit.is_none() && p.armed.is_none(), "Duress user disarms normally");
    assert!(
        p.out.iter().any(|o| matches!(o, Out::Report(r) if r.cid == 121 && r.kind == ReportKind::Alarm(AlarmKind::Duress))),
        "Duress user must silently transmit CID 121 Duress"
    );
}

#[test]
fn vista_macros_outputs_schedules_message_center_and_extras() {
    let t0 = Instant::now();
    let wall = crate::util::now_ms();
    let mut p = panel(Model::Vista20p);
    p.tick(t0, wall);

    // 1. Output devices (#7 activate / #8 deactivate)
    keys(&mut p, "1234#705", t0 + secs(1));
    assert!(p.vista_outputs[4], "output 05 must activate");
    assert!(p.lcd_vista(t0 + secs(1), wall).lines[0].contains("DEVICE 05 ON"));
    keys(&mut p, "1234#805", t0 + secs(3));
    assert!(!p.vista_outputs[4], "output 05 must deactivate");

    // 2. Macro programming (#66) + execution via *57 Macro 1 (09): bypass zone 03 + arm STAY ("603F3")
    keys(&mut p, "1234#661*603", t0 + secs(10));
    p.key(Key::Away, true, t0 + secs(11)); // inserts 'F' (single press in MacroPgm!)
    p.key(Key::Away, false, t0 + secs(11));
    keys(&mut p, "3", t0 + secs(12));
    p.key(Key::Away, true, t0 + secs(13)); // appends 'F'
    p.key(Key::Away, false, t0 + secs(13));
    p.key(Key::Away, true, t0 + secs(14)); // second D saves & exits!
    p.key(Key::Away, false, t0 + secs(14));
    assert_eq!(p.vista_macros[0], "603F3");

    p.ee.sections.insert("57".into(), "00000009".into());
    p.key(Key::Away, true, t0 + secs(20));
    p.tick(t0 + secs(22) + Duration::from_millis(100), wall);
    p.key(Key::Away, false, t0 + secs(22) + Duration::from_millis(200));
    assert!(p.lcd_vista(t0 + secs(23), wall).lines[0].contains("ENTER USER CODE"));
    keys(&mut p, "1234", t0 + secs(24));
    assert!(p.zones[2].bypassed, "Macro 1 must bypass zone 03");
    assert!(p.exit.is_some(), "Macro 1 must arm STAY");
    keys(&mut p, "12341", t0 + secs(26)); // disarm & clear bypass

    // 3. Voice Message Center (#01 Record -> MSG LED steady, 1 Stop -> flashing, #03 Play -> off)
    keys(&mut p, "#01", t0 + secs(30));
    assert_eq!(p.leds_vista()[2], Led::On, "MSG LED steady while recording");
    let memo_id = p.vista_msg_recording.unwrap();
    keys(&mut p, "1", t0 + secs(32));
    assert_eq!(p.leds_vista()[2], Led::Off, "A pending recording is not a saved message");
    let samples: std::sync::Arc<[f32]> = std::sync::Arc::from([0.13, -0.24, 0.32]);
    let clip = crate::audio::memo::Clip { samples: samples.clone(), rate: 8000 };
    p.vista_memo_result(memo_id, Ok(clip), t0 + secs(33));
    assert_eq!(p.leds_vista()[2], Led::Flash, "MSG LED flashes when memo waiting");
    keys(&mut p, "#03", t0 + secs(34));
    assert_eq!(p.leds_vista()[2], Led::Off, "MSG LED turns off after playback");
    assert!(p.out.iter().any(|out| matches!(out, Out::Audio(AudioCmd::PlayMemo(clip)) if clip.samples == samples)));
    p.out.clear();
    keys(&mut p, "#03", t0 + secs(36));
    assert!(p.out.iter().any(|out| matches!(out, Out::Audio(AudioCmd::PlayMemo(clip)) if clip.samples == samples)));

    // 4. Double-* Voice Status announces up to 3 faulted zones
    p.set_zone(1, true, false, false, t0 + secs(40));
    p.set_zone(3, true, false, false, t0 + secs(40));
    p.out.clear();
    keys(&mut p, "#0*", t0 + secs(41));
    assert!(p.out.iter().any(|o| matches!(o, Out::Audio(AudioCmd::Speak(t)) if t.contains(&"not_ready_to_arm".to_string()))));
    p.out.clear();
    keys(&mut p, "*", t0 + secs(43));
    let fault_words: Vec<String> = p.out.iter().filter_map(|o| match o { Out::Audio(AudioCmd::Speak(t)) => Some(t.clone()), _ => None }).flatten().collect();
    assert!(fault_words.iter().filter(|w| w.as_str() == "fault").count() == 2, "double-* must speak both faulted zones, got {:?}", fault_words);
    p.set_zone(1, false, false, false, t0 + secs(45));
    p.set_zone(3, false, false, false, t0 + secs(45));

    // 5. Dialer Test ([Code] + 5 + 1) displays PHONE OKAY (or COMM. FAILURE when FTC)
    keys(&mut p, "123451", t0 + secs(50));
    assert!(p.lcd_vista(t0 + secs(50), wall).lines[0].contains("PHONE OKAY"));
    p.inputs.ftc = true;
    keys(&mut p, "123451", t0 + secs(55));
    assert!(p.lcd_vista(t0 + secs(55), wall).lines[0].contains("COMM. FAILURE"));
    p.inputs.ftc = false;

    // 6. *197=2 countdown interval, *198=1 partition prefix + GoTo Partition ([Code]+*+2)
    p.ee.sections.insert("198".into(), "1".into());
    keys(&mut p, "1234*2", t0 + secs(60));
    assert_eq!(p.vista_partition, 2);
    assert!(p.lcd_vista(t0 + secs(64), wall).lines[0].starts_with("2 "));
    p.ee.sections.insert("197".into(), "2".into());
    keys(&mut p, "12342", t0 + secs(66));
    let l2 = p.lcd_vista(t0 + secs(67), wall).lines[1].clone(); // 59 s left -> rounds up to 60 in 2s step!
    assert_eq!(l2.trim_end(), "May Exit Now 60");
    keys(&mut p, "12341", t0 + secs(68));

    // 7. Schedules (#64): program Schedule 01 as Event 07 (Reminder) at current clock + 2 min & trigger!
    let target_ms = wall + 120_000;
    let tp = crate::util::parts(target_ms);
    let (h12, pm) = match tp.hour {
        0 => (12, '2'),
        1..=11 => (tp.hour, '2'),
        12 => (12, '1'),
        _ => (tp.hour - 12, '1'),
    };
    let seq = format!("1234#640107{:02}{:02}{}*1111111*100", h12, tp.minute, pm);
    keys(&mut p, &seq, t0 + secs(70));
    assert_eq!(p.vista_schedules.len(), 1);
    p.tick(t0 + secs(72), target_ms);
    assert!(p.vista_reminder_until.is_some(), "Schedule 01 Reminder must fire at matching clock time");
    assert_eq!(p.lcd_vista(t0 + secs(72), target_ms).lines[1].trim_end(), "REMINDER");

    // 8. Chime by Zone (*26 = 1): Zone 2 (Interior) chimes when enabled in chime_list!
    p.chime = true;
    p.ee.sections.insert("26".into(), "1".into());
    p.ee.sections.insert("chime_list".into(), "01000000".into());
    p.out.clear();
    p.set_zone(2, true, false, false, t0 + secs(80));
    assert!(p.out.iter().any(|o| matches!(o, Out::Audio(AudioCmd::Beeps { count: 3, .. }))), "*26 chime by zone must chime Zone 2");
}

#[test]
fn wire_protocol_pc1550_classic_bus_frames_and_led_accuracy() {
    use crate::engine::bus::*;
    use crate::engine::panel::tb;

    let t0 = Instant::now();
    let mut p = panel(Model::Pc1550);

    // 1. Disarmed ready Classic bus frame: Ready bit set, Armed/Trouble/Memory/Bypass off.
    let f0 = p.classic_bus_frame(t0);
    assert_eq!(f0.zones_byte, 0);
    assert_eq!(f0.status_byte & CLASSIC_READY, CLASSIC_READY);
    assert_eq!(f0.status_byte & (CLASSIC_ARMED | CLASSIC_MEMORY | CLASSIC_BYPASS | CLASSIC_TROUBLE), 0);

    // 2. Entering [*][5] / [*][8] / [*][9] immediately flashes the authentic Classic prompt LEDs.
    keys(&mut p, "*5", t0 + secs(1));
    let l5 = p.leds_1550(t0 + secs(1));
    assert_eq!(l5[2], Led::Flash, "PC1550 *5 NeedCode must flash Memory LED");
    assert_eq!(l5[3], Led::Flash, "PC1550 *5 NeedCode must flash Bypass LED");
    assert_eq!(l5[4], Led::Flash, "PC1550 *5 NeedCode must flash Trouble LED");
    keys(&mut p, "#", t0 + secs(2));

    keys(&mut p, "*9", t0 + secs(3));
    assert_eq!(p.leds_1550(t0 + secs(3))[1], Led::Flash, "PC1550 *9 NeedCode must flash Armed LED");
    keys(&mut p, "#", t0 + secs(4));

    // 3. Setting clock via [*][6][Master][1][HHMM] clears tb::CLOCK trouble!
    p.troubles |= tb::CLOCK;
    keys(&mut p, "*6123411430#", t0 + secs(5));
    assert_eq!(p.troubles & tb::CLOCK, 0, "PC1550 *6 clock set must clear CLOCK trouble");

    // 4. Arming -> exit delay shows BOTH Ready + Armed bits on Classic wire (dscClassic.cpp:438).
    keys(&mut p, "1234", t0 + secs(10));
    let f_exit = p.classic_bus_frame(t0 + secs(11));
    assert_ne!(f_exit.status_byte & CLASSIC_READY, 0, "Classic exit delay keeps Ready lit");
    assert_ne!(f_exit.status_byte & CLASSIC_ARMED, 0, "Classic exit delay lights Armed");

    // 5. After exit completes and an alarm triggers while armed, Memory LED turns ON while armed.
    p.set_zone(1, true, false, false, t0 + secs(15));
    p.set_zone(1, false, false, false, t0 + secs(18));
    p.tick(t0 + secs(140), 0);
    assert!(p.armed.is_some());
    p.set_zone(3, true, false, false, t0 + secs(145)); // Instant zone alarm
    p.tick(t0 + secs(146), 0);
    assert_eq!(p.leds_1550(t0 + secs(146))[2], Led::On, "Classic Memory LED must light while armed after alarm");
    assert_ne!(p.classic_bus_frame(t0 + secs(146)).status_byte & CLASSIC_MEMORY, 0);
}

#[test]
fn wire_protocol_pc1832_pk5500_status_codes_blanking_6digit_and_menus() {
    use crate::engine::bus::*;

    let t0 = Instant::now();
    let mut p = panel(Model::Pc1832);
    p.ee.sections.insert("014".into(), "67".into()); // fast 5s exit termination for test speed
    p.ee.sections.insert("015".into(), "12347".into()); // enable Quick Exit (opt 3) + Quick Arm (opt 4)

    // 1. Wire status 0x01 (Ready), 0x02 (Force-armable Zone 2 open while Ready!), 0x03 (Zone 1 open).
    let ps0 = p.powerseries_05_frame(t0);
    assert_eq!(ps0.status, 0x01);
    assert_ne!(ps0.lights & PS_LIGHT_READY, 0);
    assert_ne!(ps0.lights & PS_LIGHT_BACKLIGHT, 0);
    assert_eq!(ps0.tone_0x75, 0x00);
    p.set_zone(2, true, false, false, t0 + secs(1)); // Zone 2 is Stay/Away with force_arm = true
    assert!(p.ready());
    assert_eq!(p.powerseries_status_code(t0 + secs(1)), 0x02, "Force-armable open zone must emit status 0x02");
    assert_eq!(p.lcd_1832(t0 + secs(1), 0).lines[1].trim(), "Force Arm");
    p.set_zone(1, true, false, false, t0 + secs(2));
    assert_eq!(p.powerseries_status_code(t0 + secs(2)), 0x03);
    p.set_zone(1, false, false, false, t0 + secs(3));
    p.set_zone(2, false, false, false, t0 + secs(3));

    // 2. Wire status 0x8F: wrong code shows "Invalid / Access Code".
    keys(&mut p, "9999", t0 + secs(5));
    let inv = p.lcd_1832(t0 + secs(5), 0);
    assert_eq!(inv.lines[0].trim(), "Invalid");
    assert_eq!(inv.lines[1].trim(), "Access Code");

    // 3. Wire status 0xA3 / 0xA4 (Door Chime) and 0xAC / 0xAD (Auto-Arm) banners + Bypass scroll.
    keys(&mut p, "*4", t0 + secs(8));
    assert_eq!(p.lcd_1832(t0 + secs(8), 0).lines[1].trim(), "Feature ON");
    keys(&mut p, "*4", t0 + secs(9));
    assert_eq!(p.lcd_1832(t0 + secs(9), 0).lines[1].trim(), "Feature OFF");
    keys(&mut p, "*612342", t0 + secs(10));
    assert_eq!(p.lcd_1832(t0 + secs(10), 0).lines[0].trim(), "Auto-Arm");
    assert_eq!(p.lcd_1832(t0 + secs(10), 0).lines[1].trim(), "Enabled");
    keys(&mut p, "2#", t0 + secs(11));

    // 4. Wire status 0x33: Command Output [*][7][1] shows "Command O/P 1 / Active" -> "In Progress".
    keys(&mut p, "*71", t0 + secs(15));
    assert_eq!(p.powerseries_status_code(t0 + secs(15)), 0x33);
    assert_eq!(p.lcd_1832(t0 + secs(15), 0).lines[0].trim(), "Command O/P 1");
    p.tick(t0 + secs(18), 0); // past 2s temp_msg, still within 5s [170] PGM timer
    assert_eq!(p.lcd_1832(t0 + secs(18), 0).lines[1].trim(), "In Progress");
    p.tick(t0 + secs(22), 0); // expired

    // 5. Wire status 0x09 (Arming No Entry Delay) -> 0x06 (Armed Stay No Entry) -> 0x8A (*1) -> 0x16 (Armed Away No Entry)!
    keys(&mut p, "*91234", t0 + secs(30));
    assert_eq!(p.powerseries_status_code(t0 + secs(31)), 0x09);
    assert_eq!(p.lcd_1832(t0 + secs(31), 0).lines[0].trim(), "Arming");
    assert_eq!(p.lcd_1832(t0 + secs(31), 0).lines[1].trim(), "No Entry Delay");
    // Wait out doubled silent exit delay (240s)
    p.tick(t0 + secs(280), 0);
    let ps_stay_ne = p.powerseries_05_frame(t0 + secs(280));
    assert_eq!(ps_stay_ne.status, 0x06);
    assert_ne!(ps_stay_ne.lights & PS_LIGHT_ARMED, 0);
    assert_ne!(ps_stay_ne.lights & PS_LIGHT_BYPASS, 0);
    p.key(Key::Right, true, t0 + secs(281));
    p.key(Key::Right, false, t0 + secs(281));
    assert_eq!(p.lcd_1832(t0 + secs(281), 0).lines[0].trim(), "Armed Stay    <>");
    assert_eq!(p.lcd_1832(t0 + secs(281), 0).lines[1].trim(), "No Entry Delay");

    // 6. Wire status 0xB8: Armed [*] menu only lists the 6 armed-valid functions & rejects *5!
    keys(&mut p, "*", t0 + secs(290));
    assert_eq!(p.powerseries_status_code(t0 + secs(290)), 0xB8);
    let mut armed_labels = Vec::new();
    for step in 0..6 {
        armed_labels.push(p.lcd_1832(t0 + secs(290 + step), 0).lines[1].trim().to_string());
        p.key(Key::Right, true, t0 + secs(290 + step));
        p.key(Key::Right, false, t0 + secs(290 + step));
    }
    assert!(!armed_labels.iter().any(|s| s.contains("Access Codes")), "armed * menu must exclude Access Codes");
    assert!(armed_labels.contains(&"Quick Exit".to_string()));
    keys(&mut p, "5", t0 + secs(297)); // *5 while armed -> Function Not Available (0x8E)
    assert_eq!(p.lcd_1832(t0 + secs(297), 0).lines[0].trim(), "Function Not");

    // 7. Wire status 0x0B: Quick Exit [*][0] stays displayed during entire Quick Exit window!
    keys(&mut p, "*0", t0 + secs(300));
    assert_eq!(p.powerseries_status_code(t0 + secs(310)), 0x0B);
    assert_eq!(p.lcd_1832(t0 + secs(310), 0).lines[0].trim(), "Quick Exit in");

    // 8. Wire status 0x8A: [*][1] activates Stay/Away zones -> Armed Away No Entry (0x16)!
    p.tick(t0 + secs(430), 0); // Quick Exit expires
    keys(&mut p, "*1", t0 + secs(435));
    assert_eq!(p.lcd_1832(t0 + secs(435), 0).lines[1].trim(), "Stay/Away Zones");
    p.tick(t0 + secs(565), 0); // Stay/Away reactivation completes
    assert_eq!(p.powerseries_status_code(t0 + secs(565)), 0x16);
    keys(&mut p, "#", t0 + secs(566)); // reset scroll to 0
    p.key(Key::Right, true, t0 + secs(566));
    p.key(Key::Right, false, t0 + secs(566));
    assert_eq!(p.lcd_1832(t0 + secs(566), 0).lines[0].trim(), "Armed Away    <>");
    keys(&mut p, "1234", t0 + secs(575)); // disarm

    // 9. Wire status 0x0D (Entry Delay After Alarm) + CID 459 (Recent Closing within 2 min)!
    keys(&mut p, "1234", t0 + secs(600));
    p.set_zone(1, true, false, false, t0 + secs(602));
    p.set_zone(1, false, false, false, t0 + secs(604));
    p.tick(t0 + secs(612), 0); // armed away
    p.set_zone(3, true, false, false, t0 + secs(630)); // alarm 18s after arming -> Recent Closing CID 459!
    p.set_zone(3, false, false, false, t0 + secs(632));
    assert!(p.events.iter().any(|e| e.cid == 459), "alarm within 2 min of arming must log Recent Closing CID 459");
    // Bell cutoff expires, then door opens -> Entry Delay After Alarm (0x0D) + pulsing sounder!
    p.tick(t0 + secs(900), 0);
    p.set_zone(1, true, false, false, t0 + secs(905));
    p.tick(t0 + secs(905), 0);
    assert_eq!(p.powerseries_status_code(t0 + secs(905)), 0x0D);
    assert_eq!(p.lcd_1832(t0 + secs(905), 0).lines[1].trim(), "After Alarm");
    assert!(matches!(p.sounder_sent, Pattern::ShortShortLong { .. }), "0x0D Entry After Alarm must pulse sounder");
    keys(&mut p, "1234", t0 + secs(910)); // disarm
    p.set_zone(1, false, false, false, t0 + secs(911));
    keys(&mut p, "#", t0 + secs(912)); // exit alarm memory screen

    // 10. Wire status 0x40 / 0x17: Keypad Blanking ([016] Opt 3 & Opt 4)!
    p.ee.sections.insert("016".into(), "1345".into()); // Opt 3 blanking + Opt 4 code to wake
    p.tick(t0 + secs(945), 0); // >30s idle
    assert!(p.is_blanked_1832(t0 + secs(945)));
    assert_eq!(p.powerseries_status_code(t0 + secs(945)), 0x40);
    assert_eq!(p.lcd_1832(t0 + secs(945), 0).lines[0].trim(), "");
    assert_eq!(p.leds_5500(t0 + secs(945)), [Led::Off, Led::Off, Led::Off, Led::Off]);
    // With a trouble present, blanking switches to wire 0x17 and flashes Trouble LED!
    p.inputs.battery_low = true;
    p.tick(t0 + secs(946), 0);
    assert_eq!(p.powerseries_status_code(t0 + secs(946)), 0x17);
    assert_eq!(p.leds_5500(t0 + secs(946))[2], Led::Flash);
    // Pressing a key prompts for access code to wake (Opt 4) without arming!
    keys(&mut p, "1", t0 + secs(950));
    assert!(p.pc1832_blank_wake);
    assert_eq!(p.powerseries_status_code(t0 + secs(950)), 0x9F);
    keys(&mut p, "234", t0 + secs(951));
    assert!(!p.pc1832_blank_wake, "valid access code wakes blanked keypad");
    assert!(p.exit.is_none(), "waking a blanked keypad must NOT arm the panel");
    p.inputs.battery_low = false;
    p.ee.sections.insert("016".into(), "15".into());

    // 11. Wire status 0xBC: 6-Digit Access Codes ([701] Opt 5)!
    p.ee.sections.insert("701".into(), "5".into());
    assert_eq!(p.code_len(), 6);
    // Legacy 4-digit master code 1234 works as 123400!
    keys(&mut p, "*5123400", t0 + secs(960));
    keys(&mut p, "01", t0 + secs(961));
    assert_eq!(p.powerseries_status_code(t0 + secs(961)), 0xBC, "*5 6-digit code entry must emit wire status 0xBC");
    keys(&mut p, "654321#", t0 + secs(962));
    keys(&mut p, "654321", t0 + secs(965));
    assert!(p.exit.is_some(), "new 6-digit user code must arm PC1832");
    keys(&mut p, "654321", t0 + secs(966));
    assert!(p.exit.is_none(), "new 6-digit user code must disarm PC1832");
}

#[test]
fn wire_protocol_vista_20p_ecp_f7_frames_cycling_and_extras() {
    use crate::engine::bus::*;

    let t0 = Instant::now();
    let wall = crate::util::now_ms();
    let mut p = panel(Model::Vista20p);
    p.tick(t0, wall);

    // 1. Disarmed ready 0xF7 ECP wire frame: valid 45-byte packet with exact two's-complement checksum!
    let f7 = p.vista_f7_frame(t0, wall);
    assert_eq!(f7.beeps(), 0);
    assert_ne!(f7.byte7_status1 & ECP_B2_READY, 0);
    assert_ne!(f7.byte8_status2 & ECP_B3_AC_POWER, 0);
    assert!(f7.backlight);
    let wire = f7.to_wire_bytes([0x00, 0x00, 0x01, 0x00]); // keypad addr 16
    assert_eq!(wire[0], 0xF7);
    let cksum: u8 = wire.iter().fold(0u8, |a, &b| a.wrapping_add(b));
    assert_eq!(cksum, 0, "VISTA 0xF7 wire checksum must sum to 0 mod 256");

    // 2. [Code] + 6 + 00 clears active bypasses and logs CID 570 R.
    keys(&mut p, "1234603", t0 + secs(1));
    assert!(p.zones[2].bypassed);
    assert_ne!(p.vista_f7_frame(t0 + secs(5), wall).byte8_status2 & ECP_B3_BYPASS, 0);
    keys(&mut p, "1234600", t0 + secs(6));
    assert!(!p.zones[2].bypassed, "[Code]+6+00 must clear all bypassed zones");
    assert!(p.events.iter().any(|e| e.cid == 570 && e.qual == 'R'));

    // 3. #33 Quick-Arm Night-Stay (when *21=1) sets ECP_B1_NIGHT + ECP_B2_ARMED_STAY & reports CID 441!
    p.ee.sections.insert("21".into(), "1".into());
    p.out.clear();
    keys(&mut p, "#33", t0 + secs(10));
    let f_night = p.vista_f7_frame(t0 + secs(11), wall);
    assert_ne!(f_night.byte6_beeps_night & ECP_B1_NIGHT, 0, "#33 must set ECP NIGHT bit");
    assert_ne!(f_night.byte7_status1 & ECP_B2_ARMED_STAY, 0, "#33 must set ECP ARMED_STAY bit");
    assert!(p.out.iter().any(|o| matches!(o, Out::Report(r) if r.cid == 441)), "STAY arming must send Contact ID 441");
    keys(&mut p, "12341", t0 + secs(13));

    // 4. Multiple simultaneous troubles cycle every 2.5 s instead of hiding each other!
    p.inputs.ac_ok = false;
    p.inputs.battery_low = true;
    p.inputs.audio_ok = false;
    p.tick(t0 + secs(20), wall);
    let s0 = p.lcd_vista(t0 + secs(20), wall).lines[0].trim().to_string();
    let s1 = p.lcd_vista(t0 + secs(23), wall).lines[0].trim().to_string();
    let s2 = p.lcd_vista(t0 + secs(26), wall).lines[0].trim().to_string();
    assert_ne!(s0, s1, "multiple Vista troubles must cycle (got {} and {})", s0, s1);
    assert_ne!(s1, s2, "third Vista trouble must also cycle (got {}, {}, {})", s0, s1, s2);
    p.inputs.ac_ok = true;
    p.inputs.battery_low = false;
    p.inputs.audio_ok = true;
    p.tick(t0 + secs(28), wall);

    // 5. Exit Error sets zone_bcd = 0xEA, logs CID 374, and alternates EXIT ALARM on LCD!
    keys(&mut p, "12342", t0 + secs(30));
    p.set_zone(1, true, false, false, t0 + secs(35)); // door left open at end of exit!
    p.tick(t0 + secs(95), wall);
    assert!(p.vista_exit_alarm);
    assert_eq!(p.vista_f7_frame(t0 + secs(95), wall).zone_bcd, 0xEA, "Vista Exit Error must encode zone_bcd 0xEA");
    assert!(p.events.iter().any(|e| e.cid == 374), "Exit Error must log CID 374");
    assert_eq!(p.lcd_vista(t0 + secs(97), wall).lines[0].trim(), "EXIT ALARM");
    p.set_zone(1, false, false, false, t0 + secs(98));
    // Let entry expire into alarm, then 1st OFF latches alternating alarm memory!
    p.tick(t0 + secs(135), wall);
    assert!(p.alarm_sounding(t0 + secs(135)));
    keys(&mut p, "12341", t0 + secs(136));
    assert!(p.lcd_vista(t0 + secs(136), wall).lines[0].starts_with("ALARM 01"));
    assert!(p.lcd_vista(t0 + secs(139), wall).lines[0].contains("DISARMED"), "1st OFF must alternate ALARM XX and DISARMED");
    keys(&mut p, "12341", t0 + secs(140)); // 2nd OFF clears memory
    assert!(p.vista_alarm_mem_zone.is_none());

    // 6. Installer Programming emits ECP_B3_PROGRAM flag AND non-zero prompt_pos cursor!
    keys(&mut p, "4112800", t0 + secs(150));
    let f_pgm = p.vista_f7_frame(t0 + secs(150), wall);
    assert_ne!(f_pgm.byte8_status2 & ECP_B3_PROGRAM, 0);
    assert!(f_pgm.prompt_pos >= 16, "Programming mode must place blinking cursor on Line 2 (got {})", f_pgm.prompt_pos);
    keys(&mut p, "*99", t0 + secs(151));
}

#[test]
fn classic_utility_output_respects_the_programmed_pgm_type() {
    let now = Instant::now();
    let mut p = panel(Model::Pc1550);
    p.ee.sections.insert("24".into(), "05".into());
    keys(&mut p, "*7", now);
    assert!(p.util_until.is_none());

    p.ee.sections.insert("24".into(), "03".into());
    keys(&mut p, "*7", now + secs(1));
    assert!(p.util_until.is_none());
    keys(&mut p, "1234", now + secs(2));
    assert_eq!(p.util_until, Some(now + secs(7)));
    assert!(p.fire_reset_until.is_none());

    p.ee.sections.insert("24".into(), "04".into());
    keys(&mut p, "*7", now + secs(8));
    assert_eq!(p.fire_reset_until, Some(now + secs(13)));
}

#[test]
fn powerseries_function_keys_use_the_same_access_rules_as_the_star_menu() {
    let now = Instant::now();
    let mut p = panel(Model::Pc1832);
    p.ee.sections.insert("000.1".into(), "08".into());
    p.ee.sections.insert("015".into(), "12457".into());
    p.key(Key::Stay, true, now);
    p.tick(now + secs(3), crate::util::now_ms());
    p.key(Key::Stay, false, now + secs(3));
    assert!(matches!(p.kp, KpMode::NeedCode { purpose: crate::engine::panel::CodeFor::Bypass, .. }));

    keys(&mut p, "#", now + secs(4));
    p.ee.sections.insert("000.1".into(), "19".into());
    p.key(Key::Stay, true, now + secs(5));
    p.tick(now + secs(8), crate::util::now_ms());
    p.key(Key::Stay, false, now + secs(8));
    assert!(matches!(p.cmd_out_active, Some((3, _))));
}

#[test]
fn vista_power_up_can_start_disarmed_and_biweekly_schedule_survives_restart() {
    use chrono::{Local, TimeZone};
    let now = Instant::now();
    let model = Model::Vista20p;
    let mut p = panel(model);
    keys(&mut p, "12343", now);
    let mut saved = p.to_save(true);
    assert!(saved.armed.is_some());
    saved.ee.sections.insert("39".into(), "0".into());
    let mut p = Panel::new(model, saved, crate::config::default_labels(model), "Tester".into(), false, now);
    assert!(p.armed.is_none());

    let monday = Local.with_ymd_and_hms(2026, 11, 30, 12, 0, 0).single().unwrap().timestamp_millis();
    p.vista_schedules.push(crate::engine::panel::VistaSched {
        num: 1,
        event: 1,
        device: 2,
        hour: 12,
        days: 1 << 1,
        repeat: 2,
        ..Default::default()
    });
    p.tick(now, monday);
    assert!(p.vista_outputs[1]);
    p.vista_outputs[1] = false;
    let json = serde_json::to_string(&p.to_save(true)).unwrap();
    let mut p = Panel::new(model, serde_json::from_str(&json).unwrap(), crate::config::default_labels(model), "Tester".into(), false, now);
    p.tick(now + secs(1), monday + 1000);
    assert!(!p.vista_outputs[1], "restart during the scheduled minute must not replay the action");
    p.tick(now + secs(2), monday + 7 * 86_400_000);
    assert!(!p.vista_outputs[1], "a biweekly schedule must skip the following week");
    p.tick(now + secs(3), monday + 14 * 86_400_000);
    assert!(p.vista_outputs[1], "the schedule must fire again after two weeks");
}

#[test]
fn vista_zone_list_menu_programs_chime_and_night_stay_lists() {
    let now = Instant::now();
    let mut p = panel(Model::Vista20p);
    keys(&mut p, "4112800*81", now);
    assert!(p.is_programming());
    assert!(p.lcd_vista(now, 0).lines[0].starts_with("Zone List No."));
    // Add zones 02 and 08 to Night-Stay, then selectively remove zone 02.
    keys(&mut p, "05*02*08*00*0102*00*", now);
    assert_eq!(p.ee.raw(Model::Vista20p, "81"), "00000001");
    // Clear the Chime-by-Zone list and return to installer programming.
    keys(&mut p, "03*00*100**99", now);
    assert_eq!(p.ee.raw(Model::Vista20p, "chime_list"), "00000000");
    assert!(!p.is_programming());
}

#[test]
fn expanded_zones_use_their_own_programming_and_can_be_bypassed() {
    let now = Instant::now();
    let mut dsc = panel(Model::Pc1832);
    assert_eq!(dsc.n(), 32);
    keys(&mut dsc, "*8555500203##", now);
    assert_eq!(dsc.cfg(17).kind, ZoneKind::Instant);
    assert!(dsc.cfg(17).audible);

    let mut vista = panel(Model::Vista20p);
    assert_eq!(vista.n(), 48);
    vista.ee.sections.insert("56".into(), "0104030104090210".into());
    vista.ee.sections.insert("resp".into(), "02222222".into());
    let mut definitions = vista.ee.raw(Model::Vista20p, "56");
    assert_eq!(definitions.len(), 96);
    definitions.replace_range(94..96, "03");
    vista.ee.sections.insert("56".into(), definitions);
    assert_eq!(vista.loop_response_ms(48), 350);
    assert_eq!(vista.cfg(48).kind, ZoneKind::Instant);
    vista.set_zone(48, true, false, false, now);
    assert!(!vista.ready());
    keys(&mut vista, "1234648#", now + secs(1));
    assert!(vista.zones[47].bypassed);
}

#[test]
fn vista_periodic_test_command_programs_schedule_32() {
    let now = Instant::now();
    let mut p = panel(Model::Vista20p);
    let wall = crate::util::now_ms() / 60_000 * 60_000;
    p.tick(now, wall);
    keys(&mut p, "4112#01", now + secs(1));
    assert!(p.vista_schedules.iter().any(|schedule| schedule.num == 32 && schedule.event == 11));
    p.out.clear();
    p.tick(now + secs(2), wall + 11 * 3_600_000);
    assert!(!p.out.iter().any(|out| matches!(out, Out::Report(report) if report.cid == 602)));
    p.tick(now + secs(3), wall + 12 * 3_600_000);
    assert!(p.out.iter().any(|out| matches!(out, Out::Report(report) if report.cid == 602)));
}

#[test]
fn vista_restores_night_stay_and_manual_bypasses() {
    let now = Instant::now();
    let model = Model::Vista20p;
    let mut p = panel(model);
    p.ee.sections.insert("81".into(), "01000000".into());
    keys(&mut p, "1234603#123433", now);
    assert!(p.vista_night);
    let saved = p.to_save(true);
    let restored = Panel::new(model, saved, crate::config::default_labels(model), "Tester".into(), false, now);
    assert!(restored.vista_night);
    assert!(!restored.zones[1].bypassed, "a selected Night-Stay zone remains armed");
    assert!(restored.zones[2].bypassed && !restored.zones[2].auto_bypass, "manual bypass stays in effect");
}

#[test]
fn quiet_intrusion_keeps_phone_reports_and_fire_audio() {
    let now = Instant::now();
    let mut p = panel(Model::Vista20p);
    p.quiet_intrusion = true;
    p.ee.sections.insert("41".into(), String::new());
    p.trigger(AlarmKind::Burglary, 1, true, true, now);
    p.tick(now, crate::util::now_ms());
    assert_eq!(p.bell_sent, BellPattern::Off);
    assert_eq!(p.sounder_sent, Pattern::Off);
    let report = p.out.iter().find_map(|out| match out {
        Out::PhoneReport(report) if report.kind == ReportKind::Alarm(AlarmKind::Burglary) => Some(report),
        _ => None,
    }).expect("burglary must still be available for phone delivery");
    let cfg = crate::config::OperatorCfg { phone_alerts: true, ..Default::default() };
    let (job, snapshot) = crate::notify::for_report(report, &cfg).unwrap();
    assert_eq!(job.title, "Burglary alarm: Room Door");
    assert_eq!(job.priority, 5);
    assert!(snapshot);
    assert!(job.photo.is_none());

    p.trigger(AlarmKind::Fire, 6, true, true, now);
    p.tick(now, crate::util::now_ms());
    assert_eq!(p.bell_sent, BellPattern::Temporal3);
}

#[test]
fn optional_alerts_follow_settings_and_delivery_failures_do_not_loop() {
    let mut cfg = crate::config::OperatorCfg { phone_alerts: true, ..Default::default() };
    let mut report = Report {
        id: 1, kind: ReportKind::EntryStarted, cid: 0, qualifier: 'E',
        zone_or_user: 1, zone_label: "Door".into(), text: "Entry delay".into(),
        delay_s: 30, ts: crate::util::now_ms(),
    };
    assert!(crate::notify::for_report(&report, &cfg).is_none());
    cfg.heads_up_on_entry = true;
    assert_eq!(crate::notify::for_report(&report, &cfg).unwrap().0.title, "Door opened");
    report.kind = ReportKind::Trouble;
    report.cid = 354;
    assert!(crate::notify::for_report(&report, &cfg).is_none());
    report.cid = 380;
    assert_eq!(crate::notify::for_report(&report, &cfg).unwrap().0.title, "RoomGuard needs attention");
}

#[test]
fn two_way_home_assistant_panel_sync_controls_both_sides_without_loops() {
    use crate::engine::panel::HaPanelCmd;
    use crate::home_assistant::{AlarmState::*, PanelLink};

    for model in [Model::Pc1550, Model::Pc1832, Model::Vista20p] {
        let t0 = Instant::now();
        let mut p = panel(model);
        p.set_panel_link(PanelLink::TwoWay, t0);
        p.sync_physical_panel(Some(Disarmed), false, t0);
        p.out.clear();

        // 1. Arming from RoomGuard's keypad emits an outbound HA Arm command carrying the code.
        if model == Model::Vista20p {
            keys(&mut p, "12342", t0 + secs(1));
        } else {
            keys(&mut p, "1234", t0 + secs(1));
        }
        assert!(p.is_on(), "{model:?} should arm in TwoWay mode");
        assert!(
            p.out.iter().any(|o| matches!(
                o,
                Out::HaPanelCmd(HaPanelCmd::Arm { mode: ArmMode::Away, night: false, code: Some(c), .. }) if c == "1234"
            )),
            "{model:?} must send outbound Arm Away command to HA"
        );

        // Stale HA Disarmed within the 4s outbound grace window must NOT cancel our local arming!
        p.sync_physical_panel(Some(Disarmed), true, t0 + secs(2));
        assert!(p.is_on(), "{model:?} must ignore stale HA Disarmed right after local arm");

        // HA acknowledges arming -> armed_away: RoomGuard syncs cleanly with ZERO echoed HA commands.
        p.out.clear();
        p.sync_physical_panel(Some(Arming), true, t0 + secs(3));
        p.sync_physical_panel(Some(Away), true, t0 + secs(11));
        assert_eq!(p.armed.map(|a| a.mode), Some(ArmMode::Away));
        assert!(
            !p.out.iter().any(|o| matches!(o, Out::HaPanelCmd(_))),
            "{model:?} must never echo inbound HA state changes back to HA"
        );

        // 2. Disarming from HA disarms RoomGuard WITHOUT echoing Disarm back to HA!
        p.out.clear();
        p.sync_physical_panel(Some(Disarmed), true, t0 + secs(15));
        assert!(!p.is_on());
        assert!(!p.out.iter().any(|o| matches!(o, Out::HaPanelCmd(_))));

        // 3. Arming from HA (including Pending/Triggered) can be disarmed right on RoomGuard's keypad!
        p.sync_physical_panel(Some(Stay), true, t0 + secs(20));
        assert_eq!(p.armed.map(|a| a.mode), Some(ArmMode::Stay));
        p.sync_physical_panel(Some(Pending), true, t0 + secs(22));
        p.out.clear();
        if model == Model::Vista20p {
            keys(&mut p, "12341", t0 + secs(23));
        } else {
            keys(&mut p, "1234", t0 + secs(23));
        }
        assert!(!p.is_on(), "{model:?} keypad must disarm linked HA panel");
        assert!(
            p.out.iter().any(|o| matches!(
                o,
                Out::HaPanelCmd(HaPanelCmd::Disarm { code: Some(c) }) if c == "1234"
            )),
            "{model:?} must send outbound Disarm command with code to HA"
        );
    }

    // 4. Vista NIGHT-STAY sends night=true outbound to HA!
    let t0 = Instant::now();
    let mut v = panel(Model::Vista20p);
    v.set_panel_link(PanelLink::TwoWay, t0);
    v.sync_physical_panel(Some(Disarmed), false, t0);
    v.out.clear();
    keys(&mut v, "123433", t0 + secs(1));
    assert!(
        v.out.iter().any(|o| matches!(o, Out::HaPanelCmd(HaPanelCmd::Arm { mode: ArmMode::Stay, night: true, .. }))),
        "Vista NIGHT-STAY must send ArmNight to HA"
    );
}

#[test]
fn vista_one_time_maid_code_arms_disarms_once_then_deletes_itself() {
    let t0 = Instant::now();
    let mut p = panel(Model::Vista20p);
    // Program user 07 with code 7777 and Authority 5 (One-Time Maid/Guest).
    keys(&mut p, "12348077777", t0);
    keys(&mut p, "1234807#15", t0 + secs(2));
    assert!(p.ee.code_matches("07", "7777"));
    assert_eq!(p.ee.attrs("07") & 0x07, 5);

    // Master arms -> One-Time Maid CANNOT disarm another user's arming.
    keys(&mut p, "12343", t0 + secs(4));
    assert!(p.is_on());
    keys(&mut p, "77771", t0 + secs(6));
    assert!(p.is_on(), "one-time maid must not disarm a system armed by master");
    assert!(p.ee.has_code("07"), "code must remain intact when rejected");
    keys(&mut p, "12341", t0 + secs(8));
    assert!(!p.is_on());

    // One-Time Maid arms STAY, then disarms its own arming -> code auto-deletes!
    keys(&mut p, "77773", t0 + secs(10));
    assert!(p.is_on(), "one-time maid can arm");
    keys(&mut p, "77771", t0 + secs(12));
    assert!(!p.is_on(), "one-time maid can disarm its own arming");
    assert!(!p.ee.has_code("07"), "one-time maid code must delete itself after arm + disarm");
}

#[test]
fn pc1832_display_blank_zone_chime_hold_zero_and_numbered_alarm_memory() {
    use crate::audio::ChimeKind;
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1832);
    p.ee.sections.insert("015".into(), "12347".into()); // enable Quick Exit (opt 3) + Quick Arm (opt 4)

    // 1. [*][6][Master] -> scroll right 11 times to "Display Blank", press * to enable [016] opt 3!
    keys(&mut p, "*61234", t0);
    for step in 0..11 {
        p.key(Key::Right, true, t0 + secs(1 + step));
        p.key(Key::Right, false, t0 + secs(1 + step));
    }
    assert_eq!(p.lcd_1832(t0 + secs(12), 0).lines[1].trim(), "Display Blank");
    keys(&mut p, "*", t0 + secs(13));
    assert!(p.ee.toggle(Model::Pc1832, "016", 3), "Display Blank in *6 must enable [016] opt 3");
    assert_eq!(p.lcd_1832(t0 + secs(13), 0).lines[1].trim(), "Enabled");

    // 2. Scroll right once more to "Zone Chime", press *, pick tone 3 (Ding-Dong) on Zone 01, then disable with 0!
    p.key(Key::Right, true, t0 + secs(16));
    p.key(Key::Right, false, t0 + secs(16));
    assert_eq!(p.lcd_1832(t0 + secs(16), 0).lines[1].trim(), "Zone Chime");
    keys(&mut p, "*", t0 + secs(17));
    assert!(p.lcd_1832(t0 + secs(17), 0).lines[0].starts_with("01 Room"));
    p.out.clear();
    keys(&mut p, "3", t0 + secs(18));
    assert_eq!(p.ee.chime_sound(1), 3);
    assert!(p.cfg(1).chime);
    assert_eq!(p.lcd_1832(t0 + secs(18), 0).lines[1].trim(), "Chime: Ding-Dong");
    assert!(p.out.iter().any(|o| matches!(o, Out::Audio(AudioCmd::Chime(ChimeKind::DingDong)))));
    keys(&mut p, "0", t0 + secs(19));
    assert!(!p.cfg(1).chime, "pressing 0 in Zone Chime must disable chime on that zone");
    assert_eq!(p.lcd_1832(t0 + secs(19), 0).lines[1].trim(), "Chime: Off");
    keys(&mut p, "##", t0 + secs(20));

    // 3. Hold [0] for 2 seconds: Quick Arm when disarmed, Quick Exit when armed!
    // (Display Blank was enabled in step 1, so after 173s idle the keypad blanks and wakes on first key!)
    p.key(Key::Digit(0), true, t0 + secs(25));
    p.tick(t0 + secs(27) + Duration::from_millis(100), 0);
    p.key(Key::Digit(0), false, t0 + secs(27) + Duration::from_millis(200));
    assert!(p.exit.is_some(), "holding 0 for 2s while disarmed must Quick Arm");
    p.tick(t0 + secs(200), 0); // finish exit delay (>30s idle -> Display Blank activates!)
    assert!(p.armed.is_some());
    assert!(p.is_blanked_1832(t0 + secs(200)), "Display Blank must blank keypad after 30s idle");
    keys(&mut p, "#", t0 + secs(204)); // wake the blanked keypad first
    assert!(!p.is_blanked_1832(t0 + secs(204)), "keypress wakes the blanked keypad");
    p.key(Key::Digit(0), true, t0 + secs(205));
    p.tick(t0 + secs(207) + Duration::from_millis(100), 0);
    p.key(Key::Digit(0), false, t0 + secs(207) + Duration::from_millis(200));
    assert!(p.armed.unwrap().quick_exit.is_some(), "holding 0 for 2s while armed must start Quick Exit");

    // 4. [*][3] Alarm Memory displays exact zone number + label ("03 Window").
    p.set_zone(3, true, false, false, t0 + secs(210));
    p.set_zone(3, false, false, false, t0 + secs(212));
    keys(&mut p, "1234", t0 + secs(215));
    keys(&mut p, "#*3", t0 + secs(220));
    assert_eq!(p.lcd_1832(t0 + secs(220), 0).lines[1].trim(), "03 Window");
}

#[test]
fn pc1550_hex_mode_0_to_5_and_distinct_zone_led_flash_cadences() {
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1550);

    // 1. Hex mode in *8 programming maps * + 0..=5 -> A..=F!
    keys(&mut p, "*8150002*0123*#", t0);
    assert_eq!(p.ee.raw(Model::Pc1550, "02"), "ABCD", "PC1550 * + 0..=5 must encode hex A..=F");
    keys(&mut p, "#", t0 + secs(1));

    // 2. Distinct zone LED flash cadences:
    //    - Open zone while disarmed -> slow flash (Led::Flash)
    //    - Bypassed zone in [*][1] -> steady on (Led::On)
    //    - Alarm memory in [*][3] -> fast flash (Led::Fast)
    p.set_zone(3, true, false, false, t0 + secs(2));
    assert_eq!(p.leds_1550(t0 + secs(2))[5 + 2], Led::Flash, "open zone must slow-flash");
    p.set_zone(3, false, false, false, t0 + secs(3));

    keys(&mut p, "*13", t0 + secs(4));
    assert_eq!(p.leds_1550(t0 + secs(4))[5 + 2], Led::On, "bypassed zone in *1 must be steady ON");
    keys(&mut p, "3#", t0 + secs(5)); // unbypass & exit

    // Arm, trip instant zone 3, disarm -> [*][3] alarm memory fast-flashes zone 3!
    keys(&mut p, "1234", t0 + secs(10));
    p.tick(t0 + secs(140), 0);
    p.set_zone(3, true, false, false, t0 + secs(145));
    p.set_zone(3, false, false, false, t0 + secs(146));
    keys(&mut p, "1234", t0 + secs(148));
    assert_eq!(p.leds_1550(t0 + secs(148))[5 + 2], Led::Fast, "post-disarm alarm memory zone must fast-flash");
    keys(&mut p, "*3", t0 + secs(150));
    assert_eq!(p.leds_1550(t0 + secs(150))[5 + 2], Led::Fast, "*3 alarm memory zone must fast-flash");
}

#[test]
fn mirrored_ha_zones_display_and_chime_without_firing_duplicate_alarms() {
    use crate::home_assistant::{AlarmState, PanelLink};
    let t0 = Instant::now();
    for model in [Model::Pc1550, Model::Pc1832, Model::Vista20p] {
        let mut p = panel(model);
        p.set_panel_link(PanelLink::TwoWay, t0);
        p.ha_zone_mirror_only = true;
        p.home_zone[0] = true; // Zone 1 (Delay) comes from real panel via HA
        p.home_zone[5] = true; // Zone 6 (24h Fire) comes from real panel via HA
        p.chime = true;

        // 1. Disarmed: opening mirrored HA Zone 1 updates live keypad zone state AND chimes.
        p.out.clear();
        p.set_zone(1, true, false, false, t0 + secs(1));
        assert!(p.zones[0].open, "{model:?} mirrored HA zone must still display open");
        assert!(
            p.out.iter().any(|o| matches!(o, Out::Audio(AudioCmd::Chime(_) | AudioCmd::Speak(_)))),
            "{model:?} mirrored HA zone must still chime when disarmed"
        );
        p.set_zone(1, false, false, false, t0 + secs(2));

        // 2. Armed via linked real panel: opening mirrored HA Zone 1 or 24h Zone 6 must NOT fire duplicate local alarms/entry!
        p.sync_physical_panel(Some(AlarmState::Away), true, t0 + secs(4));
        assert!(p.armed.is_some());
        p.out.clear();
        p.set_zone(1, true, false, false, t0 + secs(6));
        p.set_zone(6, true, false, false, t0 + secs(7));
        p.tick(t0 + secs(8), 0);
        assert!(p.zones[0].open && p.zones[5].open, "{model:?} mirrored HA zones keep live open state while armed");
        assert!(p.entry.is_none(), "{model:?} mirrored HA zone must NOT start duplicate RoomGuard entry delay");
        assert!(p.alarms.is_empty(), "{model:?} mirrored HA zone must NOT fire duplicate RoomGuard alarm");

        // 3. Meanwhile, a local non-HA zone (Zone 3 Instant) STILL triggers RoomGuard's local alarm!
        p.set_zone(3, true, false, false, t0 + secs(10));
        p.tick(t0 + secs(11), 0);
        assert!(!p.alarms.is_empty(), "{model:?} non-HA local zone must still alarm normally");
    }
}

#[test]
fn ha_sensor_suggested_zone_codes_match_panel_models() {
    let make = |id: &str, name: &str, dc: Option<&str>| -> crate::home_assistant::Entity {
        let dc_json = dc.map(|d| format!(r#","device_class":"{d}""#)).unwrap_or_default();
        serde_json::from_str(&format!(
            r#"{{"entity_id":"{id}","state":"off","attributes":{{"friendly_name":"{name}"{dc_json}}}}}"#
        ))
        .expect("valid entity json")
    };
    let door = make("binary_sensor.front_door", "Front Door", Some("door"));
    let motion = make("binary_sensor.hall_pir", "Hallway Motion", Some("motion"));
    let window = make("binary_sensor.kitchen_window", "Kitchen Window", Some("window"));
    let smoke = make("binary_sensor.upstairs_smoke", "Upstairs Smoke", Some("smoke"));
    let co = make("binary_sensor.basement_co", "Basement CO", Some("carbon_monoxide"));
    let water = make("binary_sensor.sink_leak", "Sink Leak", Some("moisture"));

    assert_eq!(door.suggested_zone_code(Model::Pc1832), "01");
    assert_eq!(motion.suggested_zone_code(Model::Pc1832), "05");
    assert_eq!(window.suggested_zone_code(Model::Pc1832), "03");
    assert_eq!(smoke.suggested_zone_code(Model::Pc1832), "08");
    assert_eq!(co.suggested_zone_code(Model::Pc1832), "41");
    assert_eq!(water.suggested_zone_code(Model::Pc1832), "19");

    assert_eq!(door.suggested_zone_code(Model::Vista20p), "01");
    assert_eq!(motion.suggested_zone_code(Model::Vista20p), "04");
    assert_eq!(window.suggested_zone_code(Model::Vista20p), "03");
    assert_eq!(smoke.suggested_zone_code(Model::Vista20p), "09");
    assert_eq!(co.suggested_zone_code(Model::Vista20p), "14");
    assert_eq!(water.suggested_zone_code(Model::Vista20p), "07");

    assert_eq!(door.suggested_zone_code(Model::Pc1550), "00");
    assert_eq!(motion.suggested_zone_code(Model::Pc1550), "03");
    assert_eq!(window.suggested_zone_code(Model::Pc1550), "01");
    assert_eq!(smoke.suggested_zone_code(Model::Pc1550), "08");
    assert_eq!(co.suggested_zone_code(Model::Pc1550), "04");
    assert_eq!(water.suggested_zone_code(Model::Pc1550), "04");
}

#[test]
fn pc1550_zone_9_is_null_and_disabled_zones_stay_ready() {
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1550);
    // Disable Zone 3 by setting its zone definition digit 2 to '9' in section [11].
    p.ee.sections.insert("11".into(), "000309000308".into());
    assert_eq!(p.cfg(3).kind, ZoneKind::Null);
    p.set_zone(3, true, false, false, t0);
    assert!(p.ready(), "PC1550 disabled ('9') zone must not block Ready to Arm even if open");
}

#[test]
fn two_way_ha_blocks_arming_with_open_door_and_reconciles_drifted_disarm() {
    use crate::engine::panel::HaPanelCmd;
    use crate::home_assistant::{AlarmState::*, PanelLink};

    for model in [Model::Pc1550, Model::Pc1832, Model::Vista20p] {
        let t0 = Instant::now();
        let mut p = panel(model);
        p.set_panel_link(PanelLink::TwoWay, t0);
        p.sync_physical_panel(Some(Disarmed), false, t0);

        // 1. Open Zone 1 (Delay 1 door): Home Assistant tries to Arm Away -> MUST be blocked AND send Disarm back!
        p.set_zone(1, true, false, false, t0 + secs(1));
        assert!(!p.ready(), "{model:?} must not be ready when Zone 1 door is open");
        p.out.clear();
        p.sync_physical_panel(Some(Arming), true, t0 + secs(2));
        assert!(!p.is_on(), "{model:?} must NOT arm from HA while Zone 1 door is open");
        assert!(
            p.out.iter().any(|o| matches!(o, Out::HaPanelCmd(HaPanelCmd::Disarm { .. }))),
            "{model:?} must send Disarm back to HA to cancel arming with open door"
        );
        // Even if HA immediately jumped to Away while door is open (after outbound grace expires), still blocked!
        p.out.clear();
        p.sync_physical_panel(Some(Away), true, t0 + secs(6));
        assert!(!p.is_on(), "{model:?} must block HA Away while door is open");

        // 2. Close Zone 1 -> Arm from RoomGuard -> if HA stays Disarmed after grace window,
        //    re-syncing `Disarmed` (even though previous physical_state was already Disarmed!) MUST disarm RoomGuard!
        p.set_zone(1, false, false, false, t0 + secs(8));
        assert!(p.ready());
        p.out.clear();
        if model == Model::Vista20p {
            keys(&mut p, "12342", t0 + secs(9));
        } else {
            keys(&mut p, "1234", t0 + secs(9));
        }
        assert!(p.is_on(), "{model:?} should start arming locally");
        // After 3s outbound window expires (t0 + 13s), periodic HA refresh reports Disarmed -> RoomGuard reconciles!
        p.sync_physical_panel(Some(Disarmed), true, t0 + secs(13));
        assert!(!p.is_on(), "{model:?} must reconcile back to Disarmed when live HA state is Disarmed");
    }
}

#[test]
fn ha_triggered_and_external_siren_sound_pc_bell_and_esphome_helpers_work() {
    use crate::home_assistant::{AlarmState, Entity, PanelLink};

    // 1. AlarmState parses ESPHome DSC/Vista partition states (`ready`, `not_ready`, `armed_stay`, etc.).
    assert_eq!(AlarmState::parse("ready"), Some(AlarmState::Disarmed));
    assert_eq!(AlarmState::parse("not_ready"), Some(AlarmState::Disarmed));
    assert_eq!(AlarmState::parse("exit_delay"), Some(AlarmState::Arming));
    assert_eq!(AlarmState::parse("entry_delay"), Some(AlarmState::Pending));
    assert_eq!(AlarmState::parse("armed_instant"), Some(AlarmState::StayInstant));
    assert_eq!(AlarmState::parse("armed_maximum"), Some(AlarmState::AwayMax));

    // 2. ESPHome zone hints (`_z1`, `_z17`) and siren/bell binary_sensor detection.
    let z17: Entity = serde_json::from_str(
        r#"{"entity_id":"binary_sensor.dscalarm_top_floor_motion_z17","state":"off","attributes":{"friendly_name":"Top floor motion z17","device_class":"motion"}}"#,
    )
    .unwrap();
    assert_eq!(z17.esphome_zone_hint(32), Some(17));
    assert_eq!(z17.esphome_zone_hint(6), None);

    let esp_fire: Entity = serde_json::from_str(
        r#"{"entity_id":"binary_sensor.vistaalarm_fire_partition_1","state":"on","attributes":{"friendly_name":"Fire partition 1","device_class":"smoke"}}"#,
    )
    .unwrap();
    assert!(esp_fire.is_output());
    assert!(esp_fire.is_fire_output());
    assert!(!esp_fire.is_writable_output());

    let sun_ts: Entity = serde_json::from_str(
        r#"{"entity_id":"sensor.sun_next_dusk","state":"2026-11-27T18:44:21+00:00","attributes":{"friendly_name":"Sun Next dusk","device_class":"timestamp"}}"#,
    )
    .unwrap();
    assert!(!sun_ts.is_sensor_candidate(), "Sun/timestamp noise must be hidden from default Sensors tab");

    // 3. External HA siren (e.g. Lab siren toggle or ESPHome alarm) and HA `Triggered` state sound RoomGuard's bell!
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1832);
    p.set_panel_link(PanelLink::TwoWay, t0);
    p.sync_physical_panel(Some(AlarmState::Disarmed), false, t0);
    p.ha_external_siren = Some(BellPattern::Steady);
    p.tick(t0 + secs(1), 0);
    assert_eq!(p.bell_sent, BellPattern::Steady, "external HA siren ON must sound PC siren");
    assert!(!p.local_bell_active(t0 + secs(1)), "external HA siren must not count as local bell (loop guard)");
    p.ha_external_siren = None;
    p.tick(t0 + secs(2), 0);
    assert_eq!(p.bell_sent, BellPattern::Off, "external HA siren OFF must silence PC siren");

    p.sync_physical_panel(Some(AlarmState::Triggered), true, t0 + secs(3));
    p.tick(t0 + secs(4), 0);
    assert_eq!(p.bell_sent, BellPattern::Steady, "HA Triggered alarm panel must sound PC siren");
}

#[test]
fn lockstep_exit_and_entry_delays_stay_synced_with_real_panel() {
    use crate::engine::panel::HaPanelCmd;
    use crate::home_assistant::{AlarmState::*, PanelLink};

    for model in [Model::Pc1550, Model::Pc1832, Model::Vista20p] {
        let t0 = Instant::now();
        let mut p = panel(model);
        p.set_panel_link(PanelLink::TwoWay, t0);
        p.sync_physical_panel(Some(Disarmed), false, t0);

        // 1. RoomGuard initiates arming, real panel has a LONGER exit delay (200s > RG's local 60/120s):
        //    RoomGuard must HOLD in exit delay while HA is `Arming`, and transition to Armed at the exact
        //    moment HA transitions to `Away`!
        if model == Model::Vista20p {
            keys(&mut p, "12342", t0 + secs(1));
        } else {
            keys(&mut p, "1234", t0 + secs(1));
        }
        assert!(p.exit.is_some());
        p.sync_physical_panel(Some(Arming), true, t0 + secs(2));
        p.tick(t0 + secs(180), 0);
        assert!(
            p.exit.is_some() && p.armed.is_none(),
            "{model:?} must hold in exit delay while linked real panel is still Arming"
        );
        p.sync_physical_panel(Some(Away), true, t0 + secs(200));
        assert!(p.exit.is_none() && p.armed.is_some_and(|a| a.mode == ArmMode::Away));

        // 2. Entry delay lockstep: Door opens locally AND real panel reports Pending (longer delay):
        //    RoomGuard must NOT alarm before real panel triggers, and MUST immediately trigger on exact zone
        //    when real panel transitions to Triggered!
        p.set_zone(1, true, false, false, t0 + secs(210));
        assert!(p.entry.is_some());
        p.sync_physical_panel(Some(Pending), true, t0 + secs(211));
        p.tick(t0 + secs(260), 0); // past RG's 30s entry delay
        assert!(
            p.entry.is_some() && p.alarms.is_empty(),
            "{model:?} must hold entry delay while linked real panel is still Pending"
        );
        p.sync_physical_panel(Some(Triggered), true, t0 + secs(265));
        assert!(p.entry.is_none(), "{model:?} entry delay finishes when real panel triggers");
        assert!(
            p.alarms.iter().any(|a| a.zone == 1 && a.kind == AlarmKind::Burglary),
            "{model:?} must alarm on exact violated Zone 1 when real panel triggers"
        );

        // Disarm clears everything cleanly.
        p.set_zone(1, false, false, false, t0 + secs(266));
        p.sync_physical_panel(Some(Disarmed), true, t0 + secs(270));
        assert!(!p.is_on() && p.alarms.is_empty());

        // 3. Real panel has SHORTER exit delay (8s): RoomGuard snaps to Armed immediately at t=8s!
        if model == Model::Vista20p {
            keys(&mut p, "12342", t0 + secs(280));
        } else {
            keys(&mut p, "1234", t0 + secs(280));
        }
        p.sync_physical_panel(Some(Arming), true, t0 + secs(281));
        p.sync_physical_panel(Some(Away), true, t0 + secs(288));
        assert!(p.exit.is_none() && p.armed.is_some_and(|a| a.mode == ArmMode::Away));
        p.sync_physical_panel(Some(Disarmed), true, t0 + secs(290));

        // 4. Instant & Maximum from real panel preserve `no_entry = true`!
        p.sync_physical_panel(Some(StayInstant), true, t0 + secs(300));
        assert!(p.armed.is_some_and(|a| a.mode == ArmMode::Stay && a.no_entry));
        p.sync_physical_panel(Some(AwayMax), true, t0 + secs(305));
        assert!(p.armed.is_some_and(|a| a.mode == ArmMode::Away && a.no_entry));
        p.sync_physical_panel(Some(Disarmed), true, t0 + secs(310));
    }

    // 5. PC1832 [*][1] Stay -> Away sends outbound Arm Away to HA!
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1832);
    p.set_panel_link(PanelLink::TwoWay, t0);
    p.sync_physical_panel(Some(Stay), false, t0);
    p.out.clear();
    keys(&mut p, "*1", t0 + secs(5));
    assert!(
        p.out.iter().any(|o| matches!(o, Out::HaPanelCmd(HaPanelCmd::Arm { mode: ArmMode::Away, night: false, .. }))),
        "PC1832 [*][1] Stay->Away must forward Arm Away to HA"
    );
}

#[test]
fn two_way_bypass_sync_and_esphome_zone_status_parsing_work_both_ways() {
    use crate::engine::panel::HaBypassCmd;
    use crate::home_assistant::{self as ha, AlarmState, PanelLink};

    // 1. ESPHome `zs` parser & service/companion auto-detection.
    let snap = ha::parse_zone_status("OP:1,BY:3,BY:5,AL:2,TA:4,CK:6", 32, false);
    assert!(snap.open.contains(&1));
    assert!(snap.bypassed.contains(&3) && snap.bypassed.contains(&5));
    assert!(snap.alarmed.contains(&2));
    assert!(snap.tampered.contains(&4));
    assert!(snap.faulted.contains(&6));

    let lab = ha::parse_zone_status("1, 3, BY:5", 32, true);
    assert_eq!(lab.bypassed.iter().copied().collect::<Vec<_>>(), vec![1, 3, 5]);
    assert_eq!(ha::format_bypass_list(&[1, 3]), "BY:1,BY:3");

    let svcs = vec![
        "dscalarm_alarm_keypress_partition".to_string(),
        "dscalarm_set_alarm_state".to_string(),
    ];
    assert_eq!(
        ha::detect_esphome_prefix("sensor.dscalarm_status_partition_1", &svcs).as_deref(),
        Some("dscalarm")
    );
    assert_eq!(ha::detect_partition("sensor.dscalarm_status_partition_2"), 2);

    // 2. Real panel -> RoomGuard bypass reflection: bypassing open Zone 3 on real panel makes
    //    RoomGuard ready to arm and unbypassing restores it!
    let t0 = Instant::now();
    let mut p = panel(Model::Pc1832);
    p.set_panel_link(PanelLink::TwoWay, t0);
    p.sync_physical_panel(Some(AlarmState::Disarmed), false, t0);
    p.set_zone(3, true, false, false, t0 + secs(1));
    assert!(!p.ready(), "open Zone 3 blocks arming before bypass");

    p.sync_ha_bypasses(&[3], &[], false, true, t0 + secs(2));
    assert!(p.zones[2].bypassed && !p.zones[2].auto_bypass, "real panel BY:3 must bypass Zone 3 in RoomGuard");
    assert!(p.ready(), "bypassed open Zone 3 must allow arming");

    p.sync_ha_bypasses(&[], &[], false, true, t0 + secs(3));
    assert!(!p.zones[2].bypassed, "clearing real panel BY:3 must unbypass Zone 3 in RoomGuard");

    // 3. RoomGuard -> Real panel bypass emission (PC1832 *103# and *100#; Vista 1234603 and 1234600).
    p.out.clear();
    keys(&mut p, "*103#", t0 + secs(4));
    assert!(p.zones[2].bypassed);
    assert!(
        p.out.iter().any(|o| matches!(o, Out::HaBypassCmd(HaBypassCmd::SetZone { zone: 3, bypassed: true, .. }))),
        "PC1832 keypad bypass must emit HaBypassCmd::SetZone"
    );
    // Stale empty HA bypass refresh within 3s echo guard must NOT undo the local bypass!
    p.sync_ha_bypasses(&[], &[], false, true, t0 + secs(5));
    assert!(p.zones[2].bypassed, "echo guard must protect outbound bypass from stale HA refresh");

    p.out.clear();
    keys(&mut p, "*100#", t0 + secs(6));
    assert!(
        p.out.iter().any(|o| matches!(o, Out::HaBypassCmd(HaBypassCmd::ClearAll { .. }))),
        "PC1832 *100# must emit HaBypassCmd::ClearAll"
    );

    // 4. Physical panel fire alarm with AL:6 displays Fire Zone 6 and sounds Temporal-3!
    p.ee.sections.insert("013".into(), "35678".into()); // Temporal-3 fire enabled
    p.sync_ha_bypasses(&[], &[6], true, false, t0 + secs(10));
    p.sync_physical_panel(Some(AlarmState::Triggered), true, t0 + secs(10));
    p.tick(t0 + secs(11), 0);
    assert_eq!(p.bell_sent, BellPattern::Temporal3, "real panel fire alarm must sound Temporal-3");
    assert_eq!(p.lcd_1832(t0 + secs(11), 0).lines[0].trim(), "Fire Zone");
    assert_eq!(p.lcd_1832(t0 + secs(11), 0).lines[1].trim(), "Smoke Detector");

    // Vista bypass emission with user code preserved.
    let mut v = panel(Model::Vista20p);
    v.set_panel_link(PanelLink::TwoWay, t0);
    v.sync_physical_panel(Some(AlarmState::Disarmed), false, t0);
    v.out.clear();
    keys(&mut v, "1234603", t0 + secs(1));
    assert!(v.zones[2].bypassed);
    assert!(
        v.out.iter().any(|o| matches!(
            o,
            Out::HaBypassCmd(HaBypassCmd::SetZone { zone: 3, bypassed: true, code: Some(c) }) if c == "1234"
        )),
        "Vista bypass must emit HaBypassCmd::SetZone with 4-digit code"
    );
}

#[test]
fn cross_zoning_multi_zone_alarms_and_vista_panic_match_manuals() {
    let t0 = Instant::now();

    // 1. PC1832 Cross-Zoning ([018] opt 6 + [103]/[104] opt 9 + [176] timer) & Double-Hit ([017] opt 4).
    let mut p = panel(Model::Pc1832);
    p.ee.sections.insert("001".into(), "01050303050802040000000000000000".into()); // Z3 & Z4 Instant
    p.ee.sections.insert("018".into(), "6".into()); // Cross Zoning ON
    p.ee.sections.insert("103".into(), "123469".into()); // Z3 Cross-Zoned
    p.ee.sections.insert("104".into(), "123469".into()); // Z4 Cross-Zoned
    p.ee.sections.insert("176".into(), "030".into()); // 30s cross-zone timer
    p.arm(ArmHow::AwayKey, 40, t0);
    p.tick(t0 + secs(130), 0);
    assert!(p.armed.is_some());

    // First trip on Z3 waits (no burglary alarm yet).
    p.set_zone(3, true, false, false, t0 + secs(140));
    assert!(p.alarms.is_empty(), "single cross-zoned trip must wait for verification");
    p.set_zone(3, false, false, false, t0 + secs(142));
    // Re-tripping Z3 alone while Double-Hit ([017] opt 4) is OFF does NOT alarm!
    p.set_zone(3, true, false, false, t0 + secs(145));
    assert!(p.alarms.is_empty(), "same zone re-trip must not alarm with Double-Hit OFF");
    // Tripping Z4 within 30s verifies cross-zone and triggers BOTH Z3 and Z4!
    p.set_zone(4, true, false, false, t0 + secs(150));
    assert!(p.alarms.iter().any(|a| a.zone == 3), "cross-zone must alarm first zone (Z3)");
    assert!(p.alarms.iter().any(|a| a.zone == 4), "cross-zone must alarm second zone (Z4)");

    // With Double-Hit ON ([017] opt 4), re-tripping Z3 within 30s alarms immediately!
    keys(&mut p, "1234", t0 + secs(155));
    p.set_zone(3, false, false, false, t0 + secs(156));
    p.set_zone(4, false, false, false, t0 + secs(156));
    p.ee.sections.insert("017".into(), "14".into()); // Double-Hit ON
    p.arm(ArmHow::AwayKey, 40, t0 + secs(160));
    p.tick(t0 + secs(290), 0);
    p.set_zone(3, true, false, false, t0 + secs(300));
    assert!(p.alarms.is_empty());
    p.set_zone(3, false, false, false, t0 + secs(302));
    p.set_zone(3, true, false, false, t0 + secs(305));
    assert!(p.alarms.iter().any(|a| a.zone == 3), "Double-Hit ON must trigger on 2nd trip of same zone");

    // 2. VISTA-20P: Fire detector on Zone 08 displays FIRE 08 (NOT Zone 95!), while Keypad [A]
    //    displays FIRE 95 (configurable/disableable via panic types), *32 continuous fire, and *85 cross-zone!
    let mut v = panel(Model::Vista20p);
    let mut z56 = v.ee.raw(Model::Vista20p, "56");
    z56.replace_range(14..16, "09"); // Zone 08 = Type 09 Fire
    z56.replace_range(6..8, "03");   // Zone 04 = Type 03 Perimeter
    v.ee.sections.insert("56".into(), z56);
    v.ee.sections.insert("32".into(), "1".into()); // Fire Sounder Timeout = No timeout (continuous)
    v.ee.sections.insert("33".into(), "1".into()); // Normal bell timeout = 4 min

    // Fire sensor on Zone 08 trips -> displays FIRE 08 (never FIRE 95!) and rings past 4-min bell timeout!
    v.set_zone(8, true, false, false, t0);
    assert!(v.lcd_vista(t0, 0).lines[0].starts_with("FIRE 08"), "wired fire sensor on Zone 08 must show FIRE 08");
    v.tick(t0 + secs(600), 0); // 10 min > 4 min bell timeout
    assert_eq!(v.bell_sent, BellPattern::Temporal3, "Vista *32=1 fire sounder must stay continuous past *33 timeout");
    v.set_zone(8, false, false, false, t0 + secs(601));
    keys(&mut v, "12341", t0 + secs(602));
    keys(&mut v, "12341", t0 + secs(604));

    // Holding Keypad [A] Fire panic fires Zone 95 by default, and respects disabling ("00") in panic config!
    v.ee.sections.insert("panic".into(), "000806".into()); // 95 disabled, 96 aux, 99 silent police
    v.key(Key::Fire, true, t0 + secs(610));
    v.tick(t0 + secs(613), 0);
    v.key(Key::Fire, false, t0 + secs(613));
    assert!(v.alarms.is_empty(), "Vista Zone 95 disabled (00) must not trigger Fire panic");
    v.key(Key::Panic, true, t0 + secs(615));
    v.tick(t0 + secs(618), 0);
    v.key(Key::Panic, false, t0 + secs(618));
    assert!(
        v.alarms.iter().any(|a| a.zone == 99 && !a.audible),
        "Vista Zone 99 programmed as 06 must be 24h silent panic"
    );

    // VISTA-20P *85 Cross-Zone with Zone List 04 (Zones 3 & 4) + multi-alarm display cycling!
    keys(&mut v, "12341", t0 + secs(620));
    keys(&mut v, "12341", t0 + secs(622));
    v.ee.sections.insert("zone_list_04".into(), ((1u64 << 2) | (1u64 << 3)).to_string()); // Z3 & Z4
    v.ee.sections.insert("85".into(), "01".into()); // 30s cross-zone timer
    v.arm(ArmHow::AwayKey, 2, t0 + secs(630));
    v.tick(t0 + secs(700), 0);
    v.set_zone(3, true, false, false, t0 + secs(710));
    assert!(v.alarms.is_empty(), "Vista Zone List 04 first trip must wait for second zone");
    v.set_zone(4, true, false, false, t0 + secs(720));
    assert!(v.alarms.iter().any(|a| a.zone == 3) && v.alarms.iter().any(|a| a.zone == 4));
    let l_a = v.lcd_vista(t0 + secs(720), 0).lines[0].clone();
    let l_b = v.lcd_vista(t0 + secs(723), 0).lines[0].clone();
    assert_ne!(l_a, l_b, "Vista LCD must cycle between multiple simultaneous alarmed zones");

    // 3. PC1550 physical Arming exit beeps obey section [34] option 1 (not [12] option 5).
    let mut c = panel(Model::Pc1550);
    c.set_panel_link(crate::home_assistant::PanelLink::TwoWay, t0);
    c.sync_physical_panel(Some(crate::home_assistant::AlarmState::Arming), true, t0);
    c.tick(t0 + secs(1), 0);
    assert_eq!(c.sounder_sent, Pattern::Off, "PC1550 [34] opt 1 OFF = silent exit delay");
    c.ee.sections.insert("34".into(), "14".into());
    c.tick(t0 + secs(2), 0);
    assert_ne!(c.sounder_sent, Pattern::Off, "PC1550 [34] opt 1 ON = audible exit delay during physical Arming");
}

#[test]
fn learned_ha_timers_prevent_early_exit_skip_and_external_siren_status_works() {
    use crate::engine::panel::HaExpect;
    use crate::home_assistant::{self as ha, AlarmState::*, PanelLink};

    // 1. Entity parsing for HA manual/Alarmo `next_state`, `delay`, and `is_output_on()`.
    let manual_arming: ha::Entity = serde_json::from_str(
        r#"{"entity_id":"alarm_control_panel.roomguard_test_panel","state":"arming","attributes":{"friendly_name":"RoomGuard Test Panel","next_state":"armed_away","delay":12}}"#,
    )
    .unwrap();
    assert_eq!(manual_arming.alarm_next_state(), Some(Away));
    assert_eq!(manual_arming.alarm_delay_secs(), Some(12));

    let siren_on: ha::Entity = serde_json::from_str(
        r#"{"entity_id":"input_boolean.roomguard_lab_siren","state":"on","attributes":{"friendly_name":"Lab siren output"}}"#,
    )
    .unwrap();
    assert!(siren_on.is_output_on());

    // 2. Learning HA panel's exact exit & entry durations keeps RoomGuard's countdown in lockstep
    //    (zero skipped seconds on subsequent arms even when HA arming_time=8 != EEPROM 60/120s).
    let t0 = Instant::now();
    let mut v = panel(Model::Vista20p);
    v.ee.sections.insert("197".into(), "1".into()); // Show 1-second exit countdown on Vista LCD
    v.set_panel_link(PanelLink::TwoWay, t0);
    v.sync_physical_panel(Some(Disarmed), false, t0);

    // First cycle: HA arms with 8s exit delay -> RoomGuard learns ha_learned_exit_away = 8s.
    keys(&mut v, "12342", t0 + secs(1));
    v.sync_physical_panel_meta(Some(Arming), Some(Away), None, None, true, t0 + secs(2));
    v.sync_physical_panel_meta(Some(Away), None, None, None, true, t0 + secs(10));
    assert_eq!(v.ha_learned_exit_away, Some(8), "RoomGuard must learn HA Away exit delay (8s)");
    assert_eq!(v.armed.unwrap().user, 2, "arm_from_physical must preserve the user who armed on RoomGuard");

    // Local entry delay + HA Pending: status() must still show local Zone 1 countdown (not generic Physical entry).
    v.set_zone(1, true, false, false, t0 + secs(20));
    v.sync_physical_panel_meta(Some(Pending), None, None, None, true, t0 + secs(21));
    let (st, title, detail) = v.status(t0 + secs(22));
    assert_eq!(st, StatusKind::Entry);
    assert_eq!(title, "Entry delay");
    assert!(detail.contains("Zone 1"), "lockstep entry status must preserve local zone detail: {detail}");
    // HA triggers 8s later -> learns ha_learned_entry = 8s.
    v.sync_physical_panel_meta(Some(Triggered), None, None, None, true, t0 + secs(29));
    assert_eq!(v.ha_learned_entry, Some(8), "RoomGuard must learn HA entry delay (8s)");

    // Disarm and arm again from RoomGuard: exit countdown immediately starts at exact 8s!
    v.set_zone(1, false, false, false, t0 + secs(30));
    v.sync_physical_panel(Some(Disarmed), true, t0 + secs(31));
    keys(&mut v, "12342", t0 + secs(35));
    assert_eq!(
        v.exit.unwrap().dur,
        secs(8),
        "second arm must start directly at learned 8s so countdown hits 0 in lockstep with HA"
    );
    // When HA confirms Arming 1s later, ex.started re-anchors so network lag doesn't drift!
    v.sync_physical_panel_meta(Some(Arming), Some(Away), None, None, true, t0 + secs(36));
    assert_eq!(v.exit.unwrap().started, t0 + secs(36));
    assert_eq!(v.lcd_vista(t0 + secs(36), 0).lines[1].trim(), "May Exit Now 08");
    v.sync_physical_panel(Some(Disarmed), true, t0 + secs(40));

    // 3. HA-initiated Arming with explicit delay shows synced countdown & urgency beeps without inventing ExitRt.
    v.sync_physical_panel_meta(Some(Arming), Some(Away), Some(12), None, true, t0 + secs(50));
    assert!(v.exit.is_none());
    let (st, _, detail) = v.status(t0 + secs(54));
    assert_eq!(st, StatusKind::Exit);
    assert!(detail.contains("0:08"), "HA-initiated Arming must show live remaining seconds: {detail}");
    assert_eq!(v.lcd_vista(t0 + secs(54), 0).lines[1].trim(), "May Exit Now 08");
    v.tick(t0 + secs(54), 0); // 8s left <= 10s urgency threshold -> rapid beeps
    assert_eq!(v.sounder_sent, Pattern::Beeps { on_ms: 90, off_ms: 70, count: 1, period_ms: 160 });
    v.sync_physical_panel(Some(Disarmed), true, t0 + secs(55));

    // 4. HaExpect::Night accepts Arming (e.g. Vista Code+3+3 Night-Stay while HA is still arming).
    assert!(HaExpect::Night.matches(Arming));

    // 5. External HA siren active surfaces red Alarm status and can be silenced with Vista Code + 1!
    v.ha_external_siren = Some(BellPattern::Steady);
    v.ha_external_siren_label = Some("Lab siren output".into());
    v.tick(t0 + secs(60), 0);
    assert_eq!(v.bell_sent, BellPattern::Steady);
    let (st, title, detail) = v.status(t0 + secs(60));
    assert_eq!(st, StatusKind::Alarm);
    assert_eq!(title, "External siren active");
    assert!(detail.contains("Lab siren output"));
    v.out.clear();
    keys(&mut v, "12341", t0 + secs(62));
    assert!(
        v.out.iter().any(|o| matches!(o, Out::Disarmed { .. })),
        "Vista Code+1 while disarmed must emit Out::Disarmed to silence active external HA siren"
    );
}

#[test]
fn roomguard_reports_home_assistant_alarm_states() {
    use crate::engine::types::ArmMode;
    use crate::engine::{ha_alarm_state, notify_target};
    assert_eq!(ha_alarm_state(false, false, false, None, false), "disarmed");
    assert_eq!(ha_alarm_state(false, false, true, None, false), "arming");
    assert_eq!(ha_alarm_state(false, false, false, Some(ArmMode::Away), false), "armed_away");
    assert_eq!(ha_alarm_state(false, false, false, Some(ArmMode::Stay), false), "armed_home");
    assert_eq!(ha_alarm_state(false, false, false, Some(ArmMode::Stay), true), "armed_night");
    assert_eq!(ha_alarm_state(false, true, false, Some(ArmMode::Away), false), "pending");
    assert_eq!(ha_alarm_state(true, true, false, Some(ArmMode::Away), false), "triggered");
    assert_eq!(notify_target("notify.mobile_app_pixel"), Some(("notify".to_string(), "mobile_app_pixel".to_string())));
    assert_eq!(notify_target(" mobile_app_pixel "), Some(("notify".to_string(), "mobile_app_pixel".to_string())));
    assert_eq!(notify_target("   "), None);
    assert_eq!(notify_target("notify.bad name"), None);
}

#[test]
fn home_assistant_sensor_outages_wait_out_a_grace_period() {
    use crate::engine::sensor_outage;
    let grace = crate::engine::panel::HA_SUPERVISION_GRACE;
    let t0 = Instant::now();
    let mut since = None;
    assert_eq!(sensor_outage(&mut since, true, t0), (false, None));
    let (lost, due) = sensor_outage(&mut since, false, t0 + secs(1));
    assert!(!lost, "a brief outage keeps the sensor's last known state");
    assert_eq!(due, Some(t0 + secs(1) + grace), "and schedules one re-check");
    assert!(!sensor_outage(&mut since, false, t0 + secs(90)).0, "a Home Assistant restart is not a trouble");
    assert!(sensor_outage(&mut since, false, t0 + secs(1) + grace).0, "an outage that lasts is a supervision fault");
    assert_eq!(sensor_outage(&mut since, true, t0 + secs(200)), (false, None), "reporting again clears it");
    assert!(since.is_none());
}

#[test]
fn a_linked_panel_outage_waits_out_the_grace_period() {
    use crate::engine::panel::tb;
    let t0 = Instant::now();
    let mut p = panel(Model::Vista20p);
    p.physical_unavailable_since = Some(t0);
    p.tick(t0 + secs(90), 0);
    assert_eq!(p.troubles & tb::HOME_PANEL, 0, "no trouble beeping during a Home Assistant restart");
    p.tick(t0 + secs(125), 0);
    assert_ne!(p.troubles & tb::HOME_PANEL, 0, "a lasting outage is a trouble");
}

#[test]
fn the_lab_package_keeps_no_alarm_code() {
    // With a code here, disarming at RoomGuard sends the keypad code to Home
    // Assistant, which refuses it whenever the two differ, and Two-Way sync
    // pulls RoomGuard straight back to armed.
    let yaml = include_str!("../docs/roomguard_lab.yaml");
    assert!(yaml.lines().all(|l| !l.trim_start().starts_with("code:")));
}

#[test]
fn the_bus_bits_match_the_reference_decoders() {
    use crate::engine::bus::*;
    // dscClassic.cpp: panelData[1] bit 7 ready, bit 6 armed, bit 0 beep.
    assert_eq!((CLASSIC_READY, CLASSIC_ARMED, CLASSIC_BEEP), (0x80, 0x40, 0x01));
    // dscKeybusInterface: the 0x05 lights byte, bit 3 bypass and bit 4 trouble.
    assert_eq!((PS_LIGHT_READY, PS_LIGHT_ARMED, PS_LIGHT_BYPASS, PS_LIGHT_TROUBLE), (0x01, 0x02, 0x08, 0x10));
    // esphome-components vista.h, the F7 keypad message masks.
    assert_eq!((ECP_B1_BEEP_MASK, ECP_B1_NIGHT), (0x07, 0x10));
    assert_eq!((ECP_B2_ARMED_STAY, ECP_B2_READY, ECP_B2_CHECK_FLAG, ECP_B2_FIRE), (0x80, 0x10, 0x02, 0x01));
    assert_eq!((ECP_B3_INSTANT, ECP_B3_CHIME, ECP_B3_AC_POWER, ECP_B3_ARMED_AWAY, ECP_B3_IN_ALARM), (0x80, 0x20, 0x08, 0x04, 0x01));
}
