# Beginner's Step-by-Step Guide: Home Assistant + RoomGuard (Windows)

This guide walks you through everything from scratch, click by click. By the end, you will have:
- Home Assistant running on your Windows PC (or your existing Home Assistant box)
- Live virtual sensors (door, motion, temperature) triggering RoomGuard zones
- A live two-way alarm panel where arming or disarming in **either** RoomGuard or Home Assistant controls the other one automatically

If you **already** have Home Assistant running on a Raspberry Pi, mini PC, or VM, skip straight to **Step 2**.

---

## Step 1: Set Up Home Assistant OS in a Virtual Machine (Windows)

You can run a full copy of Home Assistant for free inside VirtualBox (or VMware) on your Windows computer.

1. Download the **Home Assistant OS image for VirtualBox (`.vdi.zip`)** from the official [Home Assistant Windows installation page](https://www.home-assistant.io/installation/windows/).
2. **Unzip the downloaded file** so you have the `.vdi` file (for example `haos_ova-14.2.vdi`). Put it in a folder you will not accidentally delete, like `Documents\HomeAssistantVM\`.
3. Open **VirtualBox** and click **New**:
   - **Name:** `HomeAssistant`
   - **Type:** `Linux`
   - **Version:** `Linux 2.6 / 3.x / 4.x / 5.x / 6.x (64-bit)` (or `Other Linux (64-bit)`)
   - **Hardware:** Give it at least **2048 MB RAM** and **2 CPUs**, and check **Enable EFI (special OSes only)**.
4. **IMPORTANT: Use the downloaded disk image (do NOT boot an empty disk!).**
   - On the **Hard Disk** screen of the wizard, pick **Use an Existing Virtual Hard Disk File**, click the folder icon, and choose the `.vdi` file you unzipped in step 2.
   - *Already created the VM with a blank disk by mistake?* Open **VM Settings -> Storage**, remove the empty `.vdi`, click **Add Hard Disk -> Add**, and pick the unzipped Home Assistant `.vdi` file (or copy the downloaded `.vdi` into your VM's folder and replace the blank disk file).
5. Open **VM Settings -> Network -> Adapter 1**:
   - Check **Enable Network Adapter**.
   - Set **Attached to:** **Bridged Adapter**.
   - Under **Name**, pick the actual Wi-Fi or Ethernet card your Windows PC uses for internet.
6. Click **Start** to boot the VM. Wait 1 to 2 minutes while it boots.
   - When it finishes, the black VM window shows the **Home Assistant** banner and an IP address like `http://192.168.1.45:8123`.
7. In Chrome or Edge on Windows, open `http://homeassistant.local:8123`.
   - If that does not load, type the exact IP address shown on the VM screen (for example `http://10.0.1.45:8123`).
   - Follow the on-screen wizard to create your Home Assistant username and password.

---

## Step 2: Create a Long-Lived Access Token and Connect RoomGuard

1. In Home Assistant, click **your name / initials** at the very bottom-left corner of the sidebar.
2. Click the **Security** tab at the top of your profile page.
3. Scroll all the way to the bottom to **Long-lived access tokens**.
4. Click **Create token**, type `RoomGuard`, and click **OK**.
5. **Copy the long token immediately** (Home Assistant only shows it once).
6. Open **RoomGuard**, click the gear icon (**Settings**), and pick **Home Assistant** in the left menu.
7. Expand **Connection** and fill in:
   - **Home Assistant URL:** `http://homeassistant.local:8123` (or your `http://192.168.x.x:8123` IP address, with no slash at the end).
   - **Long-lived access token:** paste the token you just copied.
8. Click **Connect**.
9. Look at the status card at the top of the Home Assistant tab:
   - **Spinning blue ring (`Connecting...`):** RoomGuard is signing in and loading your entities.
   - **Green checkmark (`Connected`):** Everything is live! You will see how many entities are connected.
   - **Red X (`Disconnected`):** RoomGuard lost the connection (or you clicked **Disconnect**). If the VM pauses or Wi-Fi drops, RoomGuard detects it automatically within seconds and retries on its own.
10. *(Optional)* Uncheck **Use webcam** right below Connection if you want to test purely with Home Assistant sensors first.

---

## Step 3: Install the File Editor App in Home Assistant

To add the test lab sensors and virtual test alarm panel, you need a quick way to edit two YAML files inside Home Assistant.

1. In Home Assistant, click **Settings** (bottom-left gear icon).
2. Click **Apps** (in older Home Assistant versions, this is called **Add-ons**).
3. Click the **Add-on Store / Install app** button at the bottom-right, search for **File editor**, and click **Install**.
4. Once installed, turn on **Start on boot** (and optionally **Show in sidebar**), then click **Start**.
5. Click **Open Web UI**.

---

## Step 4: Turn On the `packages` Folder in `configuration.yaml`

1. Inside **File editor**, click the **folder icon** at the top-left to open the file browser. You should see `/homeassistant/`.
2. Click **`configuration.yaml`** to open it.
3. Add these exact two lines to the file (use **2 spaces** before `packages:`, never the Tab key):

   ```yaml
   homeassistant:
     packages: !include_dir_named packages
   ```

   *(If your `configuration.yaml` already has a `homeassistant:` line, do not add a second one. Just place `  packages: !include_dir_named packages` directly underneath the existing `homeassistant:` line.)*
4. Click the red **Save** icon (top-right). Make sure the green checkmark appears in the editor bar (meaning your YAML syntax is valid).

---

## Step 5: Create the `packages` Folder and Add `roomguard_lab.yaml`

1. Click the **folder icon** (top-left of File editor) so you are looking at the `/homeassistant/` folder list again.
2. Click the **New Folder** icon at the top of the left sidebar, name it **`packages`** (all lowercase), and click **OK**.
3. Click into the new **`packages`** folder. The path at the top of the sidebar should say `/homeassistant/packages/`.
4. Now add [`roomguard_lab.yaml`](roomguard_lab.yaml) into `/homeassistant/packages/`:
   - **Option A (Upload):** Click the **Upload** icon in the sidebar and pick `docs/roomguard_lab.yaml` from your RoomGuard folder.
   - **Option B (Create & Paste):** Click the **New File** icon, name it **`roomguard_lab.yaml`** (MUST use an **underscore `_`**, NEVER a hyphen `-` or Home Assistant will fail config check!), open it, paste the full contents of [`docs/roomguard_lab.yaml`](roomguard_lab.yaml), and click **Save**.

---

## Step 6: Check Your Configuration and Restart Home Assistant

1. In Home Assistant, open **Developer tools -> YAML** (or **Settings -> System -> Check configuration** / **Tools -> YAML**).
2. Click **Check configuration**. Wait for the green message: **Configuration will not prevent Home Assistant from starting!**
   - If you get a slug/package naming error, double-check that the file inside `/homeassistant/packages/` is named `roomguard_lab.yaml` with an **underscore `_`**, not a hyphen `-`.
3. Click **Restart** -> **Restart Home Assistant**. Wait about 30 seconds.
4. Watch RoomGuard while Home Assistant restarts: RoomGuard's status card will flip to the **red X (`Disconnected`)**, then automatically reconnect and return to the **green checkmark (`Connected`)**!

---

## Step 7: Test Virtual Wireless Sensors (Door, Motion & Temperature)

1. In Home Assistant, go to **Settings -> Devices & services**, then click the **Helpers** tab at the top.
2. You will now see all 6 lab entities ready to click:
   - **Lab door open** (`input_boolean.roomguard_lab_door`) -> flips **RoomGuard Test Door** (`binary_sensor.roomguard_test_door`)
   - **Lab motion detected** (`input_boolean.roomguard_lab_motion`) -> flips **RoomGuard Test Motion** (`binary_sensor.roomguard_test_motion`)
   - **Lab temperature** (`input_number.roomguard_lab_temperature`)
   - **RoomGuard Test Panel** (`alarm_control_panel.roomguard_test_panel`)
3. In RoomGuard, open **Settings -> Home Assistant** and scroll down to **Wireless sensors -> Zones**:
   - Assign each sensor using the table below (RoomGuard fills in the recommended rule, zone name, and zone type automatically when you select a sensor!):

   | Control to flip in Home Assistant Helpers | Sensor to pick in RoomGuard | Rule | Recommended Zone |
   | --- | --- | --- | --- |
   | **Lab door open** | **RoomGuard Test Door** (`binary_sensor.roomguard_test_door`) | `Is` `on` | `01 · Room Doorway` |
   | **Lab motion detected** | **RoomGuard Test Motion** (`binary_sensor.roomguard_test_motion`) | `Is` `on` | `02 · Room Motion` |
   | **Lab temperature** | **Lab temperature** (`input_number.roomguard_lab_temperature`) | `Above` `28` | `03 · Window` |

   - **Adding more zones (Zones 09 to 48):** Need more than the 8 starter zones? Click **`+ Next unused`** right next to the zone picker (or pick any `(Unused - will enable)` slot), confirm the suggested **Zone type** and **Name zone** box, and click **Enable & assign**. RoomGuard enables that zone on the panel, names it, and connects your Home Assistant sensor in one click.
   - **Hiding unused simulated zones:** Once your real sensors are assigned, click **`Disable X unused simulated zones`** under **Zone assignments** so the left-hand **Zones** sidebar only shows your real sensors (you can re-enable any zone anytime in **Settings -> Panel** or **Settings -> Home Assistant**).

4. Now click **Lab door open** in Home Assistant Helpers and toggle it **ON** and **OFF**:
   - Watch the live dot under **Zone assignments** flip between **Closed** (green) and **OPEN** (amber).
   - Watch **Zone 1** open and close on RoomGuard's keypad! (Each zone's loop response setting adds a tiny ~0.4 second debounce just like real alarm wiring.)

---

## Step 8: How to Test the Alarm Panel (Two-Way Sync)

> **Important:** Do **NOT** assign `RoomGuard Test Panel` under *Wireless sensors -> Zones*. Alarm panels belong in the **Alarm panel sync** section!

### Connect the Test Panel in RoomGuard
1. Make sure RoomGuard is **Disarmed**.
2. Open **Settings -> Home Assistant** and find **Alarm panel sync**.
3. Open the dropdown and select **`RoomGuard Test Panel (alarm_control_panel.roomguard_test_panel)`**.
4. Pick **`Two-way sync (arm & disarm both)`**.
5. **HA panel PIN:**
   - **Recommended for real panels:** Leave **HA panel PIN** **blank**. RoomGuard forwards whatever code you type on its keypad straight to Home Assistant / ESPHome without storing your real code in plaintext config or YAML.
   - **For this local Test Lab:** Leave it blank if your RoomGuard keypad code matches the test code in `roomguard_lab.yaml` (`1234`), or enter the test lab's PIN here if you use a different code on RoomGuard's keypad. You can also move the lab PIN into Home Assistant's `secrets.yaml` (`code: !secret roomguard_test_code`) or delete the `code:` line in `roomguard_lab.yaml` completely (RoomGuard's own keypad still checks your real salted/hashed Master Code before disarming).
   - **Lockstep exit & entry timers:** Home Assistant's `manual` panel does not read your RoomGuard EEPROM delay settings, so RoomGuard automatically measures your linked panel's exact `arming_time` and `delay_time` on its first cycle and locks both countdowns together second-for-second (no skipping the last seconds!).
   - **Open doors block arming:** Just like a real wall panel, if an assigned door or window zone is open, RoomGuard blocks arming from both RoomGuard's keypad and Home Assistant until the zone is closed or bypassed.

### Test A: Arm and Disarm from Home Assistant -> Watch RoomGuard Follow
1. In Home Assistant, go to **Settings -> Devices & services -> Helpers**.
2. Click directly on **`RoomGuard Test Panel`** (a popup keypad card opens right there, no dashboard setup required!).
3. Click **Arm away** (or **Arm home** / **Arm night**).
   - Watch RoomGuard immediately enter **Exit delay** (the lab panel counts for 8 seconds), then switch to **Armed away** (or **Armed stay** / **Armed night-stay** on VistaGuard 20P)!
4. To disarm from Home Assistant: type **`1234`** in the Home Assistant popup and click **Disarm**.
   - Watch RoomGuard disarm immediately!

### Test B: Arm and Disarm from RoomGuard -> Watch Home Assistant Follow
1. Keep the `RoomGuard Test Panel` popup open in Home Assistant so you can watch it live.
2. On **RoomGuard's virtual keypad**, arm the system:
   - **RmG Classic / GuardSeries:** type **`1234`** (or hold **Stay** / **Away** on GuardSeries).
   - **VistaGuard 20P:** type **`1234` + `2`** (Away), **`1234` + `3`** (Stay), or **`1234` + `3` + `3`** (Night-Stay).
3. Watch **`RoomGuard Test Panel`** in Home Assistant immediately start arming and arm right along with RoomGuard!
4. Now disarm on **RoomGuard's virtual keypad** (`1234` on Classic/GuardSeries, or `1234` + `1` on VistaGuard 20P).
5. Watch **`RoomGuard Test Panel`** in Home Assistant immediately switch to **Disarmed**!

### Test C: Trigger an Entry Delay, Alarm, and Two-Way Bypass Sync
1. **Two-way zone bypass:** Open **Lab door open** (Zone 1) in Home Assistant so the panel shows **Not ready**.
   - On RoomGuard's keypad, bypass Zone 1 (`*101#` on GuardSeries, `*11#` on RmG Classic, or `1234` + `6` + `01` on VistaGuard 20P). Watch **`Lab bypassed zones`** (`input_text.roomguard_test_bypasses`) in Home Assistant Helpers automatically update to `BY:1`!
   - Clear or edit **`Lab bypassed zones`** in Home Assistant (for example, type `BY:1,BY:3` or clear it to blank) and watch RoomGuard immediately mirror the bypassed zones on its keypad.
2. **Entry delay & alarm:** Arm the panel from either side and wait 8 seconds until both RoomGuard and Home Assistant switch to **Armed** in lockstep (even if RoomGuard's local exit timer is longer or shorter than Home Assistant's).
3. Flip **Lab door open** ON in Home Assistant Helpers (to trigger Zone 1's entry delay and alarm, which also flips `RoomGuard Test Panel` to **Triggered**), **OR** open **Developer tools -> Actions**, run `alarm_control_panel.alarm_trigger` against `alarm_control_panel.roomguard_test_panel` to test a physical-panel entry delay and alarm.
4. Disarm from **either** RoomGuard's keypad (`1234` / `12341`) **or** Home Assistant (`1234` -> **Disarm**).

### Test D: Test Automatic Disconnect Detection
1. In VirtualBox, pause the Home Assistant VM (or click **Disconnect** in RoomGuard).
2. Within a few seconds, RoomGuard's status card turns into a **red X (`Disconnected`)** and raises a supervisory link trouble while keeping your last confirmed armed state.
3. Resume the VM (or click **Reconnect**). Watch RoomGuard show the **spinning blue ring (`Connecting...`)**, then return to the **green checkmark (`Connected`)**.

---

## Step 9: Using Real Wireless Sensors and Real Physical Alarm Panels

Once the lab works, connecting real hardware works the exact same way:

- **Wireless sensors (Zigbee, Z-Wave, Thread/Matter, Wi-Fi, ESPHome, RF):** Pair your door/window contacts, PIR motion detectors, smoke/CO detectors, water leak sensors, smart locks, or garage covers in Home Assistant first. They appear automatically in the clean, searchable **Wireless sensors -> Zones** browser (with category filters for **Sensors**, **Doors & Motion**, **Cameras**, and **All**). Click any sensor, use **`+ Next unused`** to add new zones with auto-suggested zone types and inline zone names in one click, and click **Disable unused simulated zones** to keep the sidebar focused on your actual sensors. If a wireless sensor goes offline or runs out of battery (`unavailable`), RoomGuard raises a supervisory zone fault instead of a false burglary alarm.
- **Home Assistant sirens & alarm outputs (two-way):** Under **Settings -> Home Assistant -> Sirens & alarm outputs**, click any real Zigbee/Z-Wave/Wi-Fi/ESPHome siren (`siren.*`), smart relay/plug (`switch.*`), strobe (`light.*`), or lab toggle (`input_boolean.roomguard_lab_siren`) in the dropdown (or click **`+ Quick-link Lab siren`**) to link it immediately. RoomGuard automatically turns linked outputs ON whenever RoomGuard's alarm siren sounds and turns them OFF when disarmed, AND manually turning a linked siren switch ON in Home Assistant immediately sounds RoomGuard's PC siren live (with a one-click **Silence** button in the top bar and Settings).
- **Real physical alarm panels (and avoiding conflicts):** Connect your physical panel to Home Assistant using a compatible bridge (such as Envisalink, ESPHome Keybus Interface, ESPHome ECP, or Alarmo) so it exposes an `alarm_control_panel.*` entity (or an ESPHome partition status `sensor.*` entity directly). Notice that on a real ESPHome bridge you do **not** put your real alarm code in plain YAML: Home Assistant's template panel uses `code: '{{ code }}'` so your physical panel validates whatever code you type on the keypad (and any optional ESPHome fallback stays inside `secrets.yaml` via `!secret access_code`). Under **Settings -> Home Assistant -> Alarm panel sync**, choose the mode that matches how you want RoomGuard to behave:
  - **Do not sync with a real panel (Independent):** Keeps RoomGuard 100% separate so it never touches or conflicts with your existing house alarm.
  - **Two-way sync (arm & disarm both):** Lets you arm, disarm, and bypass zones from both your real wall keypads and RoomGuard's virtual keypad. Exit and entry delays stay in lockstep with the real panel so neither side finishes early or drifts out of sync.
  - **Follow real panel only (read-only):** RoomGuard shows the real panel's state without ever sending arm or disarm commands to it.
  - **Automatic ESPHome Keybus / ECP integration:** When an ESPHome bridge exposes `sensor.*_zone_status` (`zs`), `fa_1`/`fire_1`, `armi_1`, and `esphome.<node>_alarm_keypress` / `set_alarm_state` services, RoomGuard auto-detects them to mirror real-panel bypasses, alarmed zones, Instant/Night/Away modes, and fire alarms (Temporal-3) two-way with zero extra YAML.
  - **Preventing double alarms:** When linked to a real panel whose sensors are also mapped into RoomGuard zones, check **Real panel handles alarms for Home Assistant zones (prevent double alarms)**. RoomGuard will still show live open/closed zone status, bypasses, and play Door Chime / voice chimes, while letting your real panel handle entry delays and alarms without duplicate sirens.
  - **Match your real zone list:** Open the flip-down door on your real wall keypad and copy the zone numbers, names, and types written by your installer directly into **Settings -> Panel -> Zone programming** or **Settings -> Home Assistant -> Wireless sensors -> Zones**.
