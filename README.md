# RoomGuard

RoomGuard turns your webcam into the sensors of a real alarm system. It runs faithful copies of two classic panels: an **RmG Classic PC1550** with its LED keypad and a **GuardSeries PC1832** with its LCD keypad. They follow the real installation manuals, down to the section numbers, the beeps and the factory defaults.

The camera watches your doorway and your room. A monitoring center that messages and calls you is simulated, and alerts with camera snapshots go to your phone.

## What it does

- **Two real panels.** Key sequences, lights, LCD screens, entry and exit delays, bypassing, troubles, alarm memory, user codes and installer programming all work the way the manuals describe. See [the programming reference](docs/PROGRAMMING.md).
- **Your webcam is the sensor.** The doorway is zone 1, a delay zone like a door contact. The rest of the room is zone 2, an interior zone like a motion detector. Anyone who shows up in the room without coming through the doorway sets off an instant alarm, exactly like a real interior zone.
- **It only counts real people.** See the next section.
- **It learns the room.** It works out where people come and go, which spots move by themselves (a TV, a fan, curtains) and what the door looks like shut, so an open door trips the alarm even when nobody is in view.
- **It follows people.** Every person gets a motion filter that predicts where they're heading. Pan and tilt cameras aim ahead of fast movers, and when someone gets away they check where that person was going.
- **Faces, offline.** Teach it your face and it can disarm for you during the entry delay. Nothing is uploaded.
- **Hardware that sounds right.** The keypad buzzers and four sirens (Ademco WAVE-2F, Klaxon Master Blaster, ADT RX-7C and a motor bell) are synthesized live from physical models of how each one makes sound.
- **A monitoring center.** The communicator dials out with real Contact ID or pulse format tones. An operator then messages you, calls, asks for your passcode and dispatches or cancels. Dispatching is simulated, so nobody is ever sent.
- **Alerts on your phone**, with snapshots, through the free ntfy app. No account needed.
- **Hard to turn off by accident.** Closing it while armed needs your code. After a crash or power cut it comes back armed, and it can keep the computer awake while armed.
- **No webcam? Use the demo camera.** It shows a drawn room with someone walking in and out, running through the same pipeline as a real camera.

## How it tells a person from everything else

Plain motion detection can't tell you from a flickering screen, a curtain in a draft or the camera adjusting its exposure. So RoomGuard looks for a human body.

- **A body, not a face.** One small neural network finds person shaped things. A second one fits a 17 joint skeleton, Kinect style: head, shoulders, elbows, wrists, hips, knees and ankles. Shoulders and a torso are enough, so it still sees you with your back turned or your face out of view.
- **The detector and the skeleton have to agree.** A shirt hanging on a hook is shaped like a torso, and a cat can fool the first network. Neither has a believable skeleton, so neither counts.
- **Real people move.** Nothing becomes a person until it has been seen clearly a few times and has actually moved. Posters, photos of people and coats on hooks never qualify.
- **Far away people get a closer look.** When something moves across the room, it zooms in on that spot, so it finds people the full picture is too small to show.
- **Ignored areas count for bodies too.** If you mark the TV as ignored, a person on screen isn't in your room.
- **The room behaves like a motion detector.** It reacts to people moving, not people sitting still. That's why you can stand at the keypad, see the Ready light and arm, just as you would with a real motion sensor.

The models are RTMDet and RTMPose from OpenMMLab. They run on your computer with a pure Rust engine, on their own thread, so the keypad never waits on them. The camera card shows whether body tracking is on and how long each look takes.

## Download and first launch

Get the latest build from the [Releases page](../../releases).

**Windows.** Run `roomguard-windows-x86_64.exe`. The download isn't code signed yet, so Windows SmartScreen may say "Windows protected your PC". Choose **More info**, then **Run anyway**. You only do this once.

**macOS** (Apple Silicon and Intel). Open the `.dmg` and drag RoomGuard into Applications. It isn't notarized by Apple. The first time, Control click RoomGuard and choose **Open**, or open it once and then choose **Open Anyway** in System Settings, Privacy and Security. Allow camera access when asked.

**Linux** (x86_64). Make it executable and run it:

```sh
chmod +x roomguard-linux-x86_64
./roomguard-linux-x86_64
```

The first launch walks you through setup: your name, which keypad, your master code, the camera, phone alerts and whether to open at sign in.

## Phone alerts in five minutes

RoomGuard sends alerts through [ntfy](https://ntfy.sh), a free, open source push notification service. There's no account to create.

1. In RoomGuard, turn on **Send alerts to my phone**. The setup wizard offers it, or go to **Settings**, then **Monitoring**.
2. Install the **ntfy** app on your phone. Links for Android and iPhone are on [ntfy.sh](https://ntfy.sh).
3. In the app, tap **+** and subscribe to the topic RoomGuard shows you, or scan its QR code. Leave the server as `ntfy.sh`.
4. Press **Send a test alert**. It should arrive within a few seconds.

You'll get alarms with the camera snapshot, troubles like the laptop being unplugged or the camera being covered, and a warning if RoomGuard is closed while armed. In Settings you can also ask for a heads up when an entry delay starts, or a message every time it's armed and disarmed.

Your topic name works like a password: anyone who knows it can read your alerts. RoomGuard makes a long random one, and **New topic** makes another. To keep everything on your own server, run ntfy yourself and enter its address and access token in Settings.

## Using it

### Arming and disarming

- **Arm Away:** type your code, then leave through the doorway before the exit delay ends. On the LCD keypad you can also hold **Away** for two seconds.
- **Stay home:** type your code and don't leave, or hold **Stay**. The room zone is bypassed so you can move around, and the doorway stays armed with its entry delay.
- **Night:** while armed Stay, press `*` `1`. The room zone comes back once the exit delay time has passed, like on the real panels.
- **Disarm:** come in and type your code before the entry delay runs out. If you've taught it your face, it can disarm for you.
- **Emergencies:** hold **F**, **A** or **P**. One second on the PC1550 and two on the PC1832.

### What you'll hear

The sounds follow the real keypads:

- **Each key** gives a short beep.
- **An accepted code or command** gives three quick beeps. The PC1550 beeps six times when it arms.
- **The exit delay** beeps once a second, then three times a second for the last ten seconds. A stock PC1550 is silent here. That's how it leaves the factory (section `[34]` light 1 is off), and you can turn it on.
- **The entry delay** is a steady tone, and it drops out around each key you press so you can hear every digit.
- **A wrong code** gives one long tone.

### Troubles

When something's wrong, the Trouble light comes on and the keypad gives two short beeps every 10 seconds until you press `#`. The light stays on until the problem is fixed. Press `*` `2` to see which trouble it is.

| What happened | PC1550 (`*` `2` zone light) | PC1832 (`*` `2` screen) |
| --- | --- | --- |
| Laptop battery low while unplugged | 1 | Service Required, Low Battery |
| Laptop unplugged (no beeps, just the light, like the real panels) | 2 | AC Failure |
| No working speaker or headphone output | 3 | Service Required, Bell Circuit |
| Phone alerts couldn't be delivered after eight tries | 4 | Failure to Communicate |
| A fire zone reports a fault | 5 | |
| Phone alerts are on and the internet is down | | Telephone Line |
| The webcam is unplugged or stops sending pictures | that zone's light flashes | Zone Fault |
| Something covers the webcam | that zone's light flashes | Zone Tamper |

Turning the camera off in Settings isn't a trouble, since you meant to. Unplugging it, covering it or closing its privacy shutter is.

### Bypassing a zone

Press `*` `1`. On the PC1550, press the zone number (`2` for zone 2) and its light comes on. On the PC1832, type the zone as two digits (`02` for zone 2), and the screen shows a **B** next to it. Press `#` when you're done. While a menu is open, the line under the keypad says what it's waiting for.

### The camera and the doorway

On the camera card, press **Mark the doorway** and drag a box around it. You can also let RoomGuard spot it: after a few people walk through, the **Room map** tab suggests the doorway and you just press **Use it**. You can also tell it to ignore an area, such as a TV. Until a doorway is marked, the whole view is zone 1.

### Pan and tilt cameras

Pick your camera type in **Settings**, then **Camera**: a USB pan and tilt webcam (Windows and Linux) or an OBSBOT through OBSBOT Center's OSC remote (every system). Then open **Room map** and press **Map the room** once. The camera sweeps the room, measures how far it really turns and stitches a panorama for the map.

### Keyboard

| Keys | Keypad key |
| --- | --- |
| `0` to `9`, `*`, `#` | The same keys |
| Hold `F`, `A`, `P` | Fire, auxiliary, panic |
| `Enter` or `Esc` | `#` |
| Left and right arrows | `<` and `>` (LCD keypad) |
| `F1` to `F5` | Stay, Away, Chime, Reset, Exit (LCD keypad) |

## Settings worth knowing about

- **The installer tab.** In **Settings**, **About**, tap the version number five times. An **Installer** tab appears with every programming section of the panel you're using: what it's set to, the factory value, what each option means and the exact keys to change it. It's there for when you don't have time to dig through the manual. Changes are still made at the keypad, like on the real thing.
- **Run setup again** (General, Saved data) walks through the setup wizard and keeps your codes, faces and history.
- **Start over** (General) erases everything RoomGuard saved and restarts into the setup wizard, like a fresh install. It only works while disarmed.

## Staying on guard

- **Closing while armed needs your code.** The window asks for it, or you can minimize instead. Three wrong codes in a row save a snapshot of whoever is guessing. If you program the keypad lockout (section `[36]` on the PC1550, `[012]` on the PC1832), wrong codes count toward it too.
- **Only one copy runs at a time.** Opening RoomGuard again, from a shortcut or at sign in, brings the window that's already open to the front instead of starting a second one that would fight it for the camera.
- **It comes back armed.** If RoomGuard is shut down some other way (Task Manager, a crash, a power cut), it re-arms the next time it starts and tells you on your phone, just like a real panel after a power cut.
- **Open at sign in** is an option in setup and in Settings, so it comes back after a restart.
- **Keep the computer awake while armed** stops the computer from sleeping while it's watching.

These only use ordinary, documented features. RoomGuard never hides from the task manager, never needs administrator rights, never blocks other programs and only starts at sign in if you turn that on. It uses the normal per user setting: the Run key for your account on Windows, a LaunchAgent on macOS, an autostart entry on Linux. The only reason Windows or macOS warns on first launch is that the downloads aren't code signed.

## Where your data lives

Settings, codes (salted and hashed), faces, the room map and history are saved in a folder that deleting or updating the app never touches:

- Windows: `%APPDATA%\RoomGuard`
- macOS: `~/Library/Application Support/RoomGuard`
- Linux: `~/.local/share/RoomGuard`

Saving happens in the background, so a slow disk never makes the keypad stutter. Every save goes to a temporary file that's renamed into place, so a crash can't leave a half written file, and the previous good copy is kept as a backup.

## Updates

RoomGuard checks GitHub for a newer release when it starts, shows you the release notes and asks before downloading anything. On Windows and Linux it checks that the download really is a program, replaces itself and asks you to restart. On macOS it opens the new disk image so you can drag it into Applications. Your saved data is never touched.

## Building from source

Install Rust from [rustup.rs](https://rustup.rs), then:

```sh
cargo run --release
```

On Linux, install the build packages first:

```sh
sudo apt-get install pkg-config clang libasound2-dev libudev-dev libv4l-dev \
  libxkbcommon-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libgl1-mesa-dev
```

`cargo test` runs the test suite: arming, delays, alarms, bypassing, troubles, programming, factory defaults, the communicator's Contact ID checksum, frame decoding, body tracking, room map learning, every siren model and more. GitHub Actions runs it on Windows, macOS and Linux for every push.

For a local build that can update itself, set `ROOMGUARD_REPO` to your repository (for example `kashrtx/roomguard`) before building. Release builds from GitHub Actions set it for you.

## Releasing

1. Add a section for the new version at the top of `CHANGELOG.md`.
2. Change `version` in `Cargo.toml`.
3. Push a tag, such as `git tag v2.1.0 && git push origin v2.1.0`.

The release workflow tests and builds Windows, macOS (universal) and Linux, then publishes a release with that version's changelog section as its notes. Installed copies find it through the update check.

## Coming later: more cameras, more zones

Right now one webcam covers zones 1 and 2. The plan is to let other cameras on your home network join as extra zones. Think an old phone, a spare laptop or an IP camera in the hallway, each one a door contact or a motion detector on the same panel, with its own label, zone type and bypass. The PC1832 already has the zones for it (8 here, 32 on the real panel). If you want to help shape this, open an issue.

## Honest limits

- One camera can't measure depth, so the room map is learned from what the camera sees, not a 3D floor plan.
- Doorway detection needs to watch people use the door a few times. You can always mark it by hand.
- Body tracking needs some light. In a pitch dark room the camera has nothing to see, just like you.
- Face recognition uses a classic method that works well for the same camera in the same room, but it's not as robust as big online face models. The liveness check blocks a photo held still, not a photo waved around.
- Keypad buzzers are modelled on typical ceramic piezo elements around 2.7 to 3.6 kHz, with 21 step pitch control on the GuardSeries LCD keypad. The four sirens are modelled from how the real ones are built.
- This is a fun and useful room alarm, not a certified security system. Don't rely on it to protect life or property.

## License

Copyright 2026 Kaushal Bhingaradia. Licensed under the [Apache License 2.0](LICENSE). See [NOTICE](NOTICE) for third party credits.

RoomGuard emulates how the classic PC1550 and PC1832 alarm panels behave, under its RmG Classic and GuardSeries names. It isn't made by, endorsed by or affiliated with any hardware manufacturer. All keypad graphics are drawn in code.
