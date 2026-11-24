# Programming reference

Both panels are programmed the way the real ones are: from the keypad, with
the real section numbers. Everything you change is saved per panel and kept
across updates.

Default codes: master code `1234` until you set your own during setup.
Installer code `1500` on the PC1550 (LED keypad) and `5555` on the PC1832 (LCD
keypad).

## Everyday keys (both panels)

| Keys | What it does |
| --- | --- |
| Your code | Arms when disarmed. Disarms when armed or during the entry delay. Silences a 24 hour alarm. |
| `*` `1` | Bypass zones while disarmed, then zone numbers to toggle and `#` to finish. While armed stay, brings the interior zones back (night arming). |
| `*` `2` | Trouble view. |
| `*` `3` | Alarm memory. |
| `*` `5` + master code | User codes. |
| `*` `6` + master code | User functions (clock, auto arm, chime, system test, walk test). |
| `*` `7` | Utility output and sensor reset. |
| `*` `8` + installer code | Installer programming. |
| `*` `9` + code | Arm with no entry delay. |
| `*` `0` | Quick arm when disarmed (if enabled), quick exit when armed. |
| Hold `F`, `A` or `P` | Fire, auxiliary (medical) and panic alarms. One second on the PC1550, two on the PC1832. |
| `#` | Back out of anything. Silences trouble beeps. |

On the LCD keypad, hold `Stay`, `Away`, `Chime`, `Reset` or `Exit` for two
seconds, and use `<` and `>` to scroll the screen and menus.

## Arming with the camera

- Zone 1 is the doorway, a delay zone. Without a doorway marked, it's the whole view.
- Zone 2 is the rest of the room, a stay/away zone. It only exists once a doorway is marked.
- Arm with your code and leave through the doorway: the panel arms away.
- Arm and stay inside (or hold `Stay`): zone 2 is bypassed so you can move around the room, and the doorway stays armed with its entry delay.
- Someone who appears in the room without coming through the doorway sets off an instant alarm, because an interior zone only waits if a delay zone started the entry delay first.

## PC1550 sections

Enter installer programming with `*` `8` `1500`, then a two digit section.
Zone lights 1 to 4 show the digit under the cursor in binary. `F` skips a
digit, `#` leaves a section (finished entries are kept) and leaves programming.

| Section | Contents | Default |
| --- | --- | --- |
| 11 | Zone definitions, two digits per zone (first: speed and audible, second: type 0 delay, 1 instant, 2 interior, 3 home away, 4 24 hour, 5 bell/buzzer, 6 buzzer, 7 aux delay, 8 fire) | 00 03 01 00 03 08 |
| 12 | 1st system options (toggles 1 to 6) | 3, 4, 6 on |
| 13 | 2nd system options, 3 is bell squawk | 1 on |
| 14 | 3rd system options, 1 is code required for bypass | 5 on |
| 15 | Swinger limit and burglary transmission delay | 03, 00 |
| 16 | Zones that may be bypassed | all |
| 17 | Entry, exit, bell cut off, AC delay, loop response, test cycle | 030 120 004 030 050 030 |
| 18 | Auxiliary entry and exit | 045 120 |
| 19 | Auto arm time and test time (9999 is off) | 9999 9999 |
| 20, 21, 22 | Installer, master and second master codes | 1500, 1234, blank |
| 30 | Factory default | |
| 31, 32, 34 | 4th, 5th and 6th system options | |
| 36 | Keypad lockout: wrong codes, minutes | 05 05 |

## PC1832 sections

Enter installer programming with `*` `8` `5555`, then a three digit section.
The screen shows the section name and the value. `*` switches hex entry, `<`
and `>` move the cursor, `#` leaves.

| Section | Contents | Default |
| --- | --- | --- |
| 001 | Zone definitions, two digits per zone (01 delay, 02 delay 2, 03 instant, 04 interior, 05 interior stay/away, 06 delay stay/away, 07 delayed fire, 08 fire, and more) | 01 05 03 01 05 08 02 04 |
| 005, then 01 | Entry, entry 2 and exit delays | 030 045 120 |
| 005, then 09 | Bell cut off minutes | 004 |
| 006, 007 | Installer and master codes | 5555, 1234 |
| 012 | Keypad lockout: wrong codes, minutes | 005 005 |
| 013 to 016 | System options (toggles 1 to 8), 013 option 8 is temporal fire bells | |
| 101 to 108 | Zone attributes: audible, steady, chime, bypassable, force arm | |
| 301, 310 | Phone number and account code the communicator dials | |
| 999 | Factory default (asks for the installer code and 999) | |

Keypad programming: `*` `8` `5555` then `*`. Sections 074 and 077 hold the
key and display options, 201 to 208 pick each zone's door chime sound (six
beeps, bing bing, ding dong or alarm tone).

Codes on the PC1832: `*` `5` `1234`, then a two digit code number (01 to 32,
or 40 for the master), then four digits. `*` as the first digit erases. Enter
`99` first to change a code's attributes (supervisor, duress, bypass and so
on) with the number keys.
