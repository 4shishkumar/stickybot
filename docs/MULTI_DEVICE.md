# Several sticks and Macs

StickyBot.app manages several configured devices independently. In **Devices**, use **Find nearby
devices** and add the intended stick, then **View stick** to work with its settings, conversation,
faces or firmware. **Make default** chooses the stick for hooks and tools that do not specify a
device. Changing the viewed device does not move an existing job or change this default. Pause,
resume and remove affect only that device. Removal keeps Bluetooth pairing and the stick's settings. **Where this Mac sends updates**
chooses the default, specific devices, or no devices for notifications and approval prompts.

**This Mac** shows the detected macOS computer name. A short name can contain up to 12 printable
ASCII characters, for example `Desk` or `Travel`. Leave it empty for an automatic abbreviation;
the full computer name remains visible in the dashboard. Labels identify the Mac for people;
a persistent installation identity and Bluetooth bond identify it for routing. Duplicate labels
are disambiguated on the stick and dashboard. Renaming does not require pairing again.

One stick can pair up to three Macs in shared mode. Enable it from the Devices card on supported
firmware, then pair each Mac with that same stick identity. The active Mac alone receives keyboard,
mouse and dictation input. Reconnecting a Mac does not take focus. Use the stick's Mac picker to
choose the input destination; a dashboard request requires physical confirmation on the stick.
Busy recording, playback or transfers must finish before switching. An approval can belong to a
Mac other than the input destination, and its displayed host label identifies where the decision
goes. Only the first valid approval decision resolves a prompt shown on several sticks. Permission
prompts return on the same stick after a reconnect; choosing no approval devices returns control
to the terminal immediately.

Legacy slots and bonds are retained. Shared mode uses the identity of the slot selected when it
is enabled; a Mac paired with another slot may need to pair again with this shared identity.
A reinstalled app can use legacy mode without shared-host enrollment. To clear shared records,
use the stick's **Forget saved Macs** action (press again to confirm), or send `@hosts forget=all`
over USB; `@hosts forget=<host-id>` removes one. The stick restarts, clears the corresponding
host passwords and keeps Bluetooth bonds and legacy slots. Macs can introduce themselves again.
Switch back to legacy slots for an older Mac app. The legacy Python app continues to manage one
stick and shows an explanation on Devices instead of offering unsupported multi-device controls.
Its firmware updater remains the legacy implementation; use StickyBot.app for board-aware updates.

Each shared host needs its own password assignment. The previous global slot password is not
used on newly enrolled Macs. Passwords remain write-only. In shared mode audio uses Bluetooth:
USB can be connected for charging without sending the selected Mac's dictation to the cable's Mac.

USB and TCP diagnostics also omit host names and private prompt, reply, notification and settings
payloads. They report safe event/status codes instead. An explicit USB or TCP command still gets
its normal response, only on the connection that requested it; other log viewers do not receive it.

Firmware bundles carry the board, chip, separate part offsets, size and hashes. StickyBot.app
refuses incompatible or unidentified images before writing. If a matching bundle is unavailable,
there is no update to install. Normal USB updates verify the running firmware once. For a crashed
or download-mode stick, choose its physical port and model under **USB recovery**. Recovery checks
the ROM chip before writing a matching bundle. The port you choose is the physical target even
when a different Bluetooth device is selected. Generic USB bridges are only listed as candidates;
the app does not open them during discovery. The
release packaging helper accepts `--board m5-sticks3` or `--board m5-stickc-plus2` and checks the
image's silicon ID before assigning the board name. A version folder may contain a bundle for
each board in subfolders named with these board IDs. No merged image or settings partition is
written by the app updater.

## Hardware verification before release

These checks are **pending**, require a person with two Macs and two boards, and are not replaced
by fake/native tests. The draft PR must retain `needs-hardware` until they are recorded with the
firmware build and app commit tested. No automated implementation work connected to or flashed a
real stick, started the real app or changed the person's app configuration.

- Record existing legacy pairings/settings; update both S3 and Plus2 and verify they remain intact.
- On each model, opt into shared mode, pair both Macs, and verify full/short names, duplicate-name
  suffixes, rename propagation and persistence across power cycles. Revert to legacy slots.
- Hold keys/mouse buttons, switch with the picker, and verify release reaches the old host and all
  subsequent input reaches only the chosen Mac. Disconnect/reconnect and sleep/wake either Mac.
- Send an approval from the background Mac while another has input focus. Verify the visible
  prompt owner and exactly one decision on its originating Mac, including simultaneous answers
  from two sticks and late answers after a reconnect.
- Start recordings/playback/transfers on each host/device; verify busy refusals, cancellation and
  correct reply/audio destination. Connect USB to the other Mac and verify no dictation reaches it.
- With USB attached to the other Mac and two TCP log viewers, display private prompts/events and
  verify their text and host labels are absent from diagnostics. Query `@state` from each transport
  and verify the response reaches only its requester.
- Run the Mac with both boards, pause/remove one, and verify the other's state, conversation,
  firmware job and face install continue. Change dashboard selection during a transfer.
- Offer an S3-only bundle with a Plus2 selected: verify unavailable and no write. With matching
  bundles, test Bluetooth interruption/resume, cancellation, confirmation/rollback, and selected
  USB serial-port re-enumeration with another stick also attached.
- Reinstall the app/change its installation identity in legacy mode; verify it remains usable.
  In shared mode, fill all three host records, forget one/all, then enroll again without erasing NVS.
- Capture S3 app and ROM USB serial descriptors; verify a changed serial uses only a unique newly
  appeared bootloader. Leave another bootloader attached and confirm ambiguous routing refuses.
  Recover a crashed/ROM-mode S3 and Plus2; verify a wrong model/chip refuses before any flash write.
- Disconnect/reconnect during a pending permission prompt and answer after it returns. Verify
  no-destination hooks return immediately and removing a selected destination cleans both lists.
- Capture dashboard screenshots: Devices with two boards/two named hosts, edit short name,
  background-host approval, and Plus2 update unavailable. Photograph each stick's Mac picker,
  active-host label, remote-switch confirmation and approval-owner label. Use synthetic names.
