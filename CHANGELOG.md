# Changelog

Every release lists what changed. The release workflow copies the section for
each version onto its GitHub release page.

## [2.1.0-beta.1] - 2026-11-25

> **Beta pre-release:** RoomGuard is a new project and this build is marked as a beta while remaining kinks are worked out. Feedback and bug reports are welcome!

### Smoother, no more freezes
- Saving now happens in the background. Before, every save forced a full flush to the disk on the window or panel thread, which antivirus or a synced folder could stall. That was the freezing, and it's why a keypress sometimes didn't beep until the second try.
- The laptop battery is read on its own thread, so the panel's clock never waits on slow firmware.
- Closing waits for the room map and every save to be written.
- Two slow memory leaks are gone: font sizes from window resizing, and every chat photo staying loaded forever.
- The sound engine no longer bogs down while it sits idle. Leftover numbers too tiny to hear made the CPU work dozens of times harder in every filter, which could cause crackles and late beeps.
- The keypad's lights and screen now change the instant its key beeps, instead of up to a few frames later.
- The window no longer slows down the longer you use it. History drew all of its rows on every frame and the monitoring chat drew every message, each converting its time to your time zone, so every arm and disarm made each frame a little heavier. Now only what's on screen is drawn, and the chat shows the newest 60 messages with a button for older ones.
- Body tracking runs at background priority and takes a breather between looks, so it can't hold a CPU core at 100% and heat a laptop until everything throttles.
- On Windows the panel, its sound and the window always get the CPU before background work.
- Drawing happens on the graphics card through Direct3D 12, Vulkan or Metal, falling back to OpenGL if needed. Settings has a switch to use OpenGL.
- The panel, the camera and body tracking recover from an internal error instead of stopping. Before, one rare bug could end the panel's thread and leave the keypad silent and frozen.
- The first keypress after a quiet spell beeps. An inaudible 30 Hz signal keeps speakers and headphones from powering down, it gets a little stronger as your pointer reaches the keypad, and if Windows resets the sound device, it reconnects in a moment and still plays the beep you just pressed.
- A sound output that reconnects within a few seconds no longer shows a bell circuit trouble.

### Body tracking that only sees people
- Every frame gets a full look plus up to two zoomed looks where something moved, so people far across the room are found too.
- The detector and the 17 joint skeleton have to agree, and nothing becomes a person until it's been seen clearly several times and has moved. Posters, photos and coats on hooks never count. Tested against real photos, including torso only, dark, blurred and noisy ones, and against a real bedroom full of clothes.
- Ignored areas apply to bodies, so a person on a TV you've ignored isn't in the room.
- The room zone works like a real motion detector: it follows people moving, so you can stand at the keypad, get a Ready light and arm.
- The camera card shows whether body tracking is on and how long each look takes.

### Closer to the real panels
- Factory defaults now match the manuals. On the PC1832 that's sections [012], [013], [014], [015], [017], [018], [102], [105], [106], [199], [377], [380], [381] and [702], and on the PC1550 [29] and [36]. Panels saved by 2.0.0 get the fix once, and only for sections you never changed.
- The zone attribute defaults follow the manual's table, so stay/away zones can be force armed like the real ones.
- [*][1] while armed Stay brings the room back after the exit delay time, not instantly.
- The PC1832 exit delay is silent when [014] option 6 is off, and a silent exit delay runs twice as long, up to 255 seconds.
- [014] option 8 keeps the fire bell ringing past bell cut-off instead of changing its pattern.
- Holding 1 to 5 for two seconds on the PowerSeries works like the function keys.
- Holding F, A or P closes any open menu, and function keys inside a menu give the error tone.
- Zones respect the loop response time: 500 ms on the PC1550, 400 ms on the PC1832, faster for fast zones.
- Keypresses are 80 ms, and the last digit of a code is heard before "beep beep beep" instead of being cut off.
- Feature off tones are a full 2 seconds on the PC1832.
- A force armed zone that's open when the exit delay ends is left out until it closes.
- The entry-delay keypad piezo now stops when the external burglary siren takes over, and PC1832 section [018] option 5 (Keypad Buzzer Follows Bell) controls whether the keypad buzzer follows the bell.
- On the PK5500 LCD keypad, the green Ready light turns off immediately when the exit delay begins and returns on disarm.

### Easier to use
- A line under the keypad says what the panel is waiting for in each menu, like "type the zone as two digits".
- Bypass zone numbers on the PC1832 are two digits, 02 for zone 2, like the real keypad.
- A hidden Installer tab (tap the version in About five times) shows every programming section in plain English: current value, factory value, what each option does and the keys to change it.
- Start over erases everything and restarts into the setup wizard.
- Updates check that the download really is a program before installing it, and leftover downloads are cleaned up.
- Only one copy of RoomGuard runs at a time. Opening it again brings the open window to the front, even when it's minimized. Restarting after an update or Start over still works, because the new copy waits for the old one to close.
- Three wrong codes in a row save a snapshot of whoever is guessing, even with the keypad lockout off.
- The PC1550 now shows camera troubles like the PC1832 does. Cover or unplug the webcam and the Trouble light comes on with two beeps every 10 seconds until you press #. In [*][2] the camera's zone light flashes, while the six real trouble lights keep their meanings. The status bar and zones list say whether the camera is blocked or offline.

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
