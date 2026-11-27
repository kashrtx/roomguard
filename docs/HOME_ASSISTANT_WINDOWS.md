# Test Home Assistant on Windows

Run Home Assistant OS in a virtual machine on the same Windows computer as RoomGuard. The lab package gives you a virtual door contact, motion detector, temperature slider, and alarm panel.

## Connect RoomGuard

1. Follow the Home Assistant Windows guide to import its Home Assistant OS image. In VirtualBox, enable EFI, assign at least 2 GB of RAM and two CPUs, and select **Bridged Adapter** on the network adapter you use. [Home Assistant Windows guide](https://www.home-assistant.io/installation/windows/)
2. Start the VM and open `http://homeassistant.local:8123` in your Windows browser to create your Home Assistant account. If the name fails to resolve, use the VM address shown on its console, such as `http://192.168.1.45:8123`.
3. In Home Assistant, open your profile, choose **Security**, and create a **Long-lived access token**. In RoomGuard, open **Settings → Home Assistant → Connection**, enter the same base URL and the token, then click **Connect**. [Home Assistant authentication](https://www.home-assistant.io/docs/authentication/)
4. In RoomGuard’s Home Assistant tab, turn off **Use webcam** to test with just the Home Assistant inputs. Use **Quiet intrusion alerts** under **Phone alerts** for a quieter test.

## Try a sensor without editing YAML

In Home Assistant, go to **Settings → Devices & services → Helpers → Create helper → Toggle** and name it **Test door**. With RoomGuard disarmed, search for that entity in **Settings → Home Assistant → Entities**, select **Is**, enter `on`, and assign it to **Zone 1**. Toggle the helper in Home Assistant and watch RoomGuard’s door zone open and close. The zone’s loop response setting can delay the change briefly. [Home Assistant toggle helpers](https://www.home-assistant.io/integrations/input_boolean/)

## Add the full lab

1. In Home Assistant, install and start the **File editor** app from **Settings → Apps**. Open `configuration.yaml` from the app’s file browser. [Home Assistant File editor](https://www.home-assistant.io/common-tasks/os/#installing-and-using-the-file-editor-app)
2. Create a `packages` folder beside `configuration.yaml` and put [roomguard-lab.yaml](roomguard-lab.yaml) in it. Add the following to `configuration.yaml`. If it already has a `homeassistant:` block, add `packages:` inside that block. [Home Assistant packages](https://www.home-assistant.io/docs/configuration/packages/)

   ```yaml
   homeassistant:
     packages: !include_dir_named packages
   ```

3. In Home Assistant, check the configuration under **Developer tools → YAML**, then restart Home Assistant. The helper controls appear under **Settings → Devices & services → Helpers**. You can also add them to a dashboard.

| Toggle or slider in Home Assistant | Entity to bind in RoomGuard | Rule |
| --- | --- | --- |
| Lab door open | `binary_sensor.roomguard_test_door` | Is `on` |
| Lab motion detected | `binary_sensor.roomguard_test_motion` | Is `on` |
| Lab temperature | `input_number.roomguard_lab_temperature` | Above `28` |

To test following, choose **RoomGuard Test Panel** under **Follow physical panel**. Add the Home Assistant alarm entity to a dashboard or operate it from **Developer tools → Actions**. Arm Home or Away and watch the eight-second exit phase. Once armed, run `alarm_control_panel.alarm_trigger` against the test panel to see entry delay followed by an alarm. Disarm the test panel in Home Assistant with **1357**. The RoomGuard master code is separate. [Home Assistant Manual alarm panel](https://www.home-assistant.io/integrations/manual/)

To test a lost connection, pause the VM while the test panel is armed. Once RoomGuard detects the disconnect, it shows the link fault and keeps its last reported arming mode. Resume the VM, wait for the link to return, then disarm from Home Assistant.
