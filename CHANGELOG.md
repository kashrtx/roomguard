# Changelog

Every release lists what changed. The release workflow copies the section for
each version onto its GitHub release page.

## [2.0.0] - 2026-11-24

A complete rewrite as a native desktop app in Rust for Windows, macOS and Linux.

### Panels
- Two panels emulated from their manuals: the DSC PC1550 with an LED keypad and the DSC PowerSeries PC1832 with a 2 x 16 LCD keypad.
- Real key sequences and menus: bypass, trouble view, alarm memory, user codes and attributes, user functions, quick arm, quick exit, night arming, door chime, sensor reset, command outputs and the event buffer.
- Installer programming with the real section numbers, including toggle options, data sections, hex entry and factory reset.
- Entry and exit delays, interior follower zones, stay and away arming, swinger shutdown, keypad lockout, auto arm, duress codes and fire, auxiliary and panic keys.
- Troubles from the computer itself: unplugging is an AC failure, a low laptop battery is a low battery, missing speakers are a bell trouble, and a covered or unplugged camera is a zone tamper or fault.
- The panel comes back armed after the app is closed, crashes or loses power, like a real panel.

### Camera
- The webcam is the room's sensors: the doorway is zone 1 and the rest of the room is zone 2.
- Multi person tracking with a Kalman filter that predicts where each person is heading.
- A learned room map: doorways from foot traffic, walking paths, and spots that move on their own.
- Door watching: an open door trips the doorway zone even with nobody in view.
- Pan and tilt support for USB PTZ webcams and OBSBOT cameras, with lead aiming, search plans when someone gets away, and a room mapping sweep that calibrates the motor and stitches a panorama.
- Offline face recognition that can disarm during the entry delay, with a check that blocks a photo held still.
- A demo camera for trying everything without a webcam.

### Sound
- Keypad buzzers and four sirens synthesized from physical models: Ademco WAVE-2F, Klaxon Master Blaster, ADT RX-7C and a motor bell.
- Optional phone line sounds of the communicator dialing out with Contact ID or pulse format.

### Monitoring
- A simulated monitoring center that messages and calls you, asks for your passcode, and dispatches or cancels. Dispatching is simulated.
- Phone alerts with camera snapshots through ntfy, retried eight times before a communication trouble shows.

### App
- A first run setup that covers your name, keypad, master code, camera, phone alerts and sign in.
- Closing while armed asks for your code. Wrong codes count toward the keypad lockout.
- Optional opening at sign in, and keeping the computer awake while armed.
- Everything is saved to a data folder that updates and reinstalls never touch, with crash safe writes and a backup copy.
- An update checker that shows the release notes and asks before installing.
- Licensed under the Apache License 2.0.

## Earlier browser versions

### 1.1.1

- **Fixed: face teaching froze on the first pose.** A leftover beep call from the sound rewrite crashed it on every capture, silently, so it never got past the first view.
- **Only one bridge can run.** On Windows a second copy could quietly share the port, so saving failed at random and an old copy could serve an old app. Starting the bridge again now just opens the app, and an older bridge still running is reported.
- **Face teaching is easier.** It waits for the room map instead of fighting the sweep, works from further away, zooms in on you with a moving camera, and says why if it can't see a face.
- **Finds small, far-away faces.** When it sees a person's body but no face, it looks closely at their head at full resolution.
- **Saving survives a bridge restart.** The app fetches the bridge's new key and retries.

### 1.1.0

- **Whole-body detection.** People are tracked by body as well as face. It tilts up from a tall person's body to find their face, keeps knowing who someone is while they're turned around, and a person whose face it never sees still trips the alarm (zone 02) and gets a full-body photo.
- **Room mapping replaces the old calibration.** The first start sweeps across the room in steps, calibrating from many measurements and stitching a panorama. Later starts do a quick check, and re-map automatically if the camera turns differently.
- **Room map under the video.** Shows the view, entry way, people, last sighting and heading. Click to look somewhere; drag on it to mark the entry.
- **Smarter searching.** Leads fast runners further, investigates movement while searching, and sweeps the whole room from the side someone ran toward before going home. "Keep searching for" sets how long.
- **Fixed:** a crash when following someone by face whose face briefly left the picture; posters' bodies counting as alive when the frame edge cut them off.

### 1.0.0

- First release: face recognition with liveness checks, two original alarm keypads with realistic piezo sounds, zones, user and duress codes, programming, entry way and ignore areas, gimbal following with direction-of-travel search, recordings, and automatic saving through the bridge.
