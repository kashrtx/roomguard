# Programming reference

All three panels are programmed the way the real ones are: from the keypad, with the real section and field numbers. Everything you change is saved per panel and kept across updates.

You don't have to memorize any of this. Open **Settings -> Panel** for plain-English controls (delays, chime, quick arm, zone types, buzzer/voice levels) synced live with the keypad, or open **Settings -> About** and tap the version number five times to reveal the full raw **Installer** section/field table.

Default codes: master code `1234` until you set your own during setup. Installer code `1500` on **RmG Classic (`PC1550`)**, `5555` on **GuardSeries (`PC1832`)**, and `4112` on **VistaGuard 20P (`VISTA-20P`)**.

## Everyday keys (both panels)

| Keys | What it does |
| --- | --- |
| Your code | Arms when disarmed, disarms when armed or during the entry delay, and silences a 24 hour alarm. |
| `*` `1` | Bypass zones while disarmed. Then the zone number (one digit on the PC1550, two digits on the PC1832, so `02` for zone 2) and `#` to finish. PC1832 extras: `00` clears all, `99` recalls the last group, `95` saves a group, `91` recalls it. PC1550: `9` recalls the last group. While armed Stay, `*` `1` brings the room zone back once the exit delay time has passed (night arming). |
| `*` `2` | Trouble view. |
| `*` `3` | Alarm memory. |
| `*` `4` | PC1832: door chime on (three beeps) or off (a two second tone). PC1550: the downloading call. |
| `*` `5` + master code | User codes. |
| `*` `6` + master code | User functions: clock, auto arm, chime on the PC1550, system test, and on the LCD keypad the event buffer, brightness, contrast and the 21 step buzzer pitch. |
| `*` `7` | PC1550: utility output, which from the factory is the 5 second smoke detector reset. PC1832: command outputs, `*` `7` `2` is sensor reset. |
| `*` `8` + installer code | Installer programming. |
| `*` `9` + code | Arm with no entry delay. The Armed light flashes. |
| `*` `0` | Quick arm when disarmed (if enabled), quick exit when armed (if enabled). |
| Hold `F`, `A` or `P` | Fire, auxiliary (medical) and panic. One second on the PC1550, two on the PC1832. Holding one also closes any open menu. |
| `#` | Back out of anything, and silence trouble beeps. |

On the GuardSeries LCD keypad, hold **Stay**, **Away**, **Chime**, **Reset** or **Exit** for two seconds. Holding the digits `1` to `5` for two seconds does the same, exactly like the real PowerSeries keypads. Function keys don't work inside a `*` menu, and the keypad gives its error tone. Use `<` and `>` to scroll the screen and menus.

## How it sounds

| What happens | PC1550 | PC1832 |
| --- | --- | --- |
| A key | short beep | short beep |
| Code or command accepted | 3 quick beeps (6 when arming) | 3 quick beeps |
| Wrong code | one long tone, 1.5 seconds | one long tone, 2 seconds |
| Exit delay | silent from the factory; with `[34]` light 1, a pulsing tone that speeds up for the last 10 seconds | a beep a second, then three a second for the last 10 seconds (`[014]` option 6, on from the factory) |
| Exit delay when arming with Stay | | silent and twice as long, up to 255 seconds; `[022]` option 8 makes it beep |
| Entry delay | steady tone; `[34]` light 4 pulses it for the last 10 seconds | steady tone, pulsing for the last 10 seconds |
| A key during the entry delay | the tone drops out around each beep | the tone drops out around each beep |
| Door chime | 5 beeps | 6 beeps, or the sound picked for that zone |
| Troubles | 2 short beeps every 10 seconds until `#`; losing AC power is silent | the same; AC is silent unless `[018]` option 8 is on |

## Troubles

| Trouble | What sets it in RoomGuard | PC1550 `*` `2` | PC1832 `*` `2` |
| --- | --- | --- | --- |
| Low battery | Laptop unplugged and under 20% | light 1 | 1, Service Required |
| AC failure | Laptop unplugged. Silent from the factory, light only. | light 2 | 2 |
| Bell circuit | No working audio output | light 3 | 1, Service Required |
| Failure to communicate | Phone alerts failed eight times. Clears when one gets through or when you leave `*` `2`. | light 4 | 4 |
| Fire loop trouble | A fire zone reports a fault | light 5 | |
| Loss of time | Never in RoomGuard, the computer keeps the time | light 6 | 8 |
| Telephone line | Phone alerts on and the internet is down | | 3 |
| Zone fault | The webcam stops sending pictures | that zone's light flashes | 5 |
| Zone tamper | The webcam is covered | that zone's light flashes | 6 |

The real PC1550 only has the six troubles on its zone lights, so RoomGuard shows camera troubles by flashing the light of the zone that camera feeds. That way the six steady lights keep their real meanings.

## Arming with the camera

- Zone 1 is the doorway, a delay zone. Without a doorway marked, it's the whole view.
- Zone 2 is the rest of the room, a stay/away zone. It only exists once a doorway is marked.
- The room acts like a real motion detector: it reacts to people moving, not people sitting still. Stand still at the keypad and you get a Ready light.
- Arm with your code and leave through the doorway: the panel arms Away.
- Arm and stay inside, or hold **Stay**: zone 2 is bypassed so you can move around, and the doorway stays armed with its entry delay.
- Someone who appears in the room without coming through the doorway sets off an instant alarm, because an interior zone only waits if a delay zone started the entry delay first.
- A zone has to stay open for the loop response time before the panel believes it: 500 ms on the PC1550 (section `[17]`, or 10 ms for fast zones), 400 ms on the PC1832 (36 ms for fast zones in `[030]`).

## RmG Classic PC1550 sections (v4.1)

Enter installer programming with `*` `8` `1500`, then a two digit section. Zone lights 1 to 4 show the digit under the cursor in binary. `F` skips a digit, `*` switches to hex entry, and `#` leaves a section (finished entries are kept) and then leaves programming. Programming times out after two minutes without a key.

| Section | Contents | Default |
| --- | --- | --- |
| 01 to 04 | 1st and 2nd phone numbers and account codes | 5551234 and 1234, so RoomGuard's monitoring center hears you. Blank from the factory. |
| 05 to 10 | Reporting codes for zone alarms, restorals, openings, closings, priority and maintenance | zone alarms 31 to 36 (RoomGuard), the rest 00 |
| 11 | Zone definitions, two digits per zone. First: 0 slow audible, 1 slow silent, 2 fast audible, 3 fast silent. Second: 0 delay, 1 instant, 2 interior, 3 home-away, 4 24 hour bell, 5 24 hour bell/buzzer, 6 24 hour buzzer, 7 auxiliary delay, 8 fire. | 00 03 01 00 03 08. RoomGuard makes zone 2 the room. The factory has 00 01 01 02 02 08. |
| 12 | 1st system options | lights 3, 4 and 6 on |
| 13 | 2nd system options (2 master locked, 3 bell squawk, 5 code 6 is a one time maid's code) | light 1 on |
| 14 | 3rd system options (1 code needed to bypass, 2 `*` `4` downloading, 5 panic beeps) | light 5 on |
| 15 | Swinger shutdown and burglary transmission delay | 03, 00 |
| 16 | Which zones can be bypassed | all six |
| 17 | Entry delay, exit delay, bell cut-off, AC report delay, slow loop response, test cycle | 030 120 004 030 050 030 |
| 18 | Auxiliary delay entry and exit | 045 120 |
| 19 | Auto arm time and test time (`9999` is off) | 9999 9999 |
| 20, 21, 22 | Installer, master and second master codes | 1500, 1234, blank |
| 23 | Communication formats | 1 1 |
| 24 | PGM output (`04` is the 5 second reset pulse, `03` makes `*` `7` need a code) | 04 |
| 25 | Communicator call directions | 1111 |
| 26, 27, 28 | Downloading phone number, access code and panel ID | blank, 1515, 1501 |
| 29 | Rings before answering | 12 |
| 30 | Reset to factory default | |
| 31 | 4th system options (4 bypass status while armed) | all off |
| 32 | 5th system options (3 home-away zones get an entry delay, 4 quick exit, 6 standard pulsed fire, off means Temporal Three) | lights 3 and 6 on |
| 33 | Answering machine double call timer | 060 |
| 34 | 6th system options (1 audible exit delay with urgency, 2 squawk on exit, 3 audible exit fault, 4 entry urgency, 5 squawk on entry, 6 `F` key off) | light 4 on (entry urgency in last 10s) |
| 35 | LINKS1000 test reporting code | FF |
| 36 | Keypad lockout: wrong codes, minutes | 00 00, which is off |
| 90, 91 | Installer's lockout on and off | |

## GuardSeries PC1832 sections (v4.6)

Enter installer programming with `*` `8` `5555`, then a three digit section. The screen shows the section's name and value. `*` switches to hex entry, `<` and `>` move the cursor, and `#` leaves.

| Section | Contents | Default |
| --- | --- | --- |
| 000 | Keypad enrollment (`0` partition and slot, `1` to `5` what each function key does) | 11; 03 04 06 14 16 |
| 001 | Zone definitions, two digits per zone (`01` delay 1, `02` delay 2, `03` instant, `04` interior, `05` stay/away, `06` delay stay/away, `07` delayed fire, `08` fire, `09` to `21` 24 hour types, `25` interior delay, `26` 24 hour non-alarm, `30` fire supervisory, `31` day zone, `32` instant stay/away, `35` 24 hour bell/buzzer, `36` non-latching tamper, `37` night zone, `41` carbon monoxide). Changing a zone sets its `[101]` to `[108]` attributes to that type's defaults. | 01 05 03 01 05 08 02 04. RoomGuard makes zone 2 the room. The factory has 01 03 03 03 04 04 04 04. |
| 005 | System times: `[01]` partition 1 entry 1, entry 2 and exit, `[02]` partition 2, `[09]` bell cut-off | 030 045 120; 004 |
| 006, 007, 008 | Installer, master and maintenance codes (the maintenance code only arms) | 5555, 1234, blank |
| 009 to 011 | PGM outputs | 19 10 |
| 012 | Keypad lockout: wrong codes, minutes | 000 000, which is off |
| 013 | 1st system options (3 all troubles while armed, 5 auto arm schedule in `*` `6`, 6 audible exit fault, 7 event buffer follows swinger, 8 Temporal Three fire) | 3, 5, 6 and 7 on |
| 014 | 2nd system options (1 to 5 bell squawks, 6 audible exit with urgency, 7 exit delay termination, 8 continuous residential fire bell) | 6 on |
| 015 | 3rd system options (1 fire key, 2 panic audible, 3 quick exit, 4 quick arm, 5 code to bypass, 6 master locked, 7 TLM, 8 TLM audible when armed) | 1, 2, 4 and 7 on |
| 016 | 4th system options (1 AC trouble shown, 5 backlight, 7 bypass status while armed) | 1 and 5 on |
| 017 | 5th system options | 1 on |
| 018 | 6th system options (5 keypad buzzer follows bell, 7 exit delay restart, 8 AC trouble beeps) | all off |
| 019, 021, 022, 023 | 7th to 10th system options (`019` 7 `*` `6` for all users, `021` 1 codes blocked during entry, `022` 1 code for `*` `1`, `*` `2`, `*` `3`, `022` 8 audible Stay exit, `023` 1 fire key beeps only, `023` 7 silent trouble beeps) | all off |
| 030 | Fast loop response, zones 1 to 8 | all off, 400 ms |
| 101 to 108 | Zone attributes: 1 audible, 2 steady, 3 chime, 4 bypassable, 5 force arm, 6 swinger shutdown, 7 transmission delay, 8 wireless | per zone type, from the manual's table |
| 170, 176, 199 | PGM timer, cross zone timer, auto arm pre-alert | 005, 060, 004 |
| 201, 202 | Partition mask and partition 1 zones | 1; zones 1 to 8 |
| 301 to 303, 310, 311 | Phone numbers and account codes | 5551234 and 1234 for RoomGuard's monitoring center |
| 350 | Communicator formats | 03 03, Contact ID, which RoomGuard's monitoring center listens for. The factory has 04 04, SIA FSK. |
| 377 | Communication variables (swinger limits, communication delay, AC report delay, TLM delay, test cycle) | 003 003 003 000 030 010 030 030 007 |
| 380, 381, 382 | Communicator options (`380` 1 communications on, `381` 4 closing confirmation) | `380` 1; `381` 5 and 6; `382` all off |
| 701, 702 | International options | `701` all off; `702` 2 on |
| 900 | Shows the panel version (`0460`) | |
| 901 | Installer's walk test on or off (a 2 second bell burst for each zone trip) | |
| 902, 903 | Reset and view module supervision | |
| 989 | Master code back to `1234` (`989` plus the installer code) | |
| 990, 991 | Installer's lockout on and off | |
| 999 | Factory default (`999` plus the installer code plus `999`) | |

Keypad programming: `*` `8` `5555`, then `*`. Sections `074`, `076` and `077` hold key and display options, and `201` to `208` pick each zone's door chime sound (six beeps, bing bing, ding dong or alarm tone).

User codes on the PC1832: `*` `5` `1234`, then a two digit user number (`01` to `39`, `40` is the master, `41` to `71`), then four digits. `*` as the first digit erases. Enter `99` first to change a user's attributes (1 supervisor, 2 duress, 3 bypass, 7 bell squawk, 8 one time use).

## VistaGuard 20P fields (`VISTA-20P` / `VG6160V`)

Enter installer programming with `4112` `8` `0` `0`. Type `*field` to edit a field, `#field` to review a field read-only, `*97` to restore factory defaults, `*98` to exit with installer lockout, or `*99` to exit normally.

| Field | Contents | Default |
| --- | --- | --- |
| `*20` | Installer code (4 digits) | `4112` |
| `*21` | Quick arm enable (`#` + Away/Stay/Max/Instant without a code) | `0` (off) |
| `*23` | Quick / forced bypass (`[Code] + 6 + #`) | `0` (off) |
| `*26` | Chime by zone | `0` |
| `*32`, `*33` | Fire sounder timeout and Bell timeout (`1` = 4 min, `2` = 8 min, `3` = 12 min, `4` = 16 min) | `0`, `1` |
| `*34`, `*35`, `*36` | Exit delay, Entry delay 1, Entry delay 2 (seconds; `97` = 120 s) | `60`, `30`, `30` |
| `*37` | Audible exit warning (slow beeps, ramping to rapid beeps in the last 10 s) | `1` (on) |
| `*38` | Confirmation of arming ding on external bell | `0` (off) |
| `*41`, `*42`, `*43` | Primary phone, secondary phone, Partition 1 account code | `5551234`, blank, `1234` |
| `*50` | Burglary dialer delay (`0` none, `1` 15 s, `2` 30 s, `3` 45 s) | `2` (30 s) |
| `*56` | Zone types (2 digits per zone: `01` entry/exit 1, `02` entry/exit 2, `03` perimeter, `04` interior follower, `05` trouble/day, `06`/`07`/`08` 24 h, `09` fire, `10` interior w/ delay, `14` CO, `16` fire w/ verify) | `01 04 03 01 04 09 02 10` |
| `*57` | Function keys A-D (2 digits per key; `00` default panic/page, `01` paging, `02` Display Time/Date, `03` Away [Key D default], `04` Stay, `05` Night-Stay, `06` Step arming, `07` Output toggle, `08` Comm test, `09`–`12` Macros 1–4) | `00 00 00 03` |
| `*59` | Exit error report code | `0` |
| `*81` | Night-Stay zones (zone list 05, 1 digit per zone: `0`/`1`) | `00000000` |
| `*84` | Auto-Stay arming (converts Away to Stay, or Maximum to Instant, if no door opens during exit) | `3` (on) |
| `*86` | Cancel verify display (`Alarm Canceled` when clearing alarm memory) | `1` (on) |
| `*90` | Event log enables (`00`–`15` bitmask: `1` alarms, `2` troubles, `4` bypasses, `8` open/close) | `15` (all) |
| `*91` | Option selection (bit `8` enables exit delay restart via `*` or reopening the door) | `8` |
| `*93` | Reports in armed period (swinger limit) | `1` |
| `*197` | Exit time display interval (`0` = `You may exit now`, `1`–`5` = live seconds countdown step) | `0` (off) |
| `*198` | Display partition number prefix on idle LCD (`0` off, `1` on) | `0` (off) |

Advanced 6160V keypad commands:
- **Voice & Message Center:** `#` `0` `2` voice volume (`3` louder, `6` quieter, `4` voice status/chime on/off, `#` save); `#` `0` `*` or `#` `*` voice status (press `*` again within 10 s to announce up to 3 faulted zones); `#` `0` `1` record voice memo (`1` stops, flashing `MSG` LED); `#` `0` `3` play & clear voice memo.
- **Time & Date (`#63`):** `[Master]` + `#` + `63` shows `TIME/DATE` for 30 s; press `*` while disarmed to edit (`HH:MM`, `1`=PM/`2`=AM, `YY/MM/DD`, `*` forward/save at last digit, `#` back). Program `*57` option `02` for one-touch Time/Date viewing without a code.
- **100-Event History Log (`#60`):** `[Master/Installer]` + `#` + `60` displays events in Contact ID format (`001 E401 U002 P1` / timestamp); `*` = older, `#` = newer.
- **Schedules (`#64`):** `[Master/Installer]` + `#` + `64` configures schedules (`01`–`16` user, `17`–`32` installer) for Output On (`01`), Forced Stay (`04`), Forced Away (`05`), Auto Disarm (`06`), Reminder (`07`), or Periodic Test (`11`). `[Installer]` + `#` + `0` + `0`/`1`/`2` sets 24h / 7d / 28d test intervals.
- **Macros (`#66`) & Outputs (`#7`/`#8`):** `[Master]` + `#` + `66` programs Macros 1–4 (`AWAY`/D inserts `F` separator and saves on second press); `[Code]` + `#` + `7`/`8` + `[01-18]` activates/deactivates virtual outputs.
- **User Attributes:** `[Master]` + `8` + `[03-49]` + `#` + `0` (delete), `#` + `1` + `0`–`4` (authority: `0` Standard, `1` Arm-Only, `2` Guest, `3` Duress CID 121, `4` Partition Master), `#` + `2` + `0`–`8` (access group), `#` + `3` + `0` + `1`–`3` + `#` (partition), `#` + `5` + `0`/`1` (pager). `[Installer]` + `8` + `02` + `[4-digit]` sets the Master code in a single entry.
- **Partition Commands:** `[Code]` + `*` + `0`–`3` GoTo Partition; `[Code]` + `0` + `1`/`2`/`3`/`4`/`7` Multi-Partition global command.

## Updating a panel saved by an older RoomGuard

RoomGuard 2.0.0 shipped some sections that didn't match the manuals. The first time 2.1.0 loads a saved panel, it sets those sections to the real factory values, but only the ones still holding the old shipped value, so anything you programmed yourself stays. The event history lists which sections changed. Factory reset (section `[30]` on the PC1550, `[999]` on the PC1832) gets you the factory values for everything.
