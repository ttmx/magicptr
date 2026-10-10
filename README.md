# magicptr

Point and click on an Android TV box with an LG Magic Remote.

Over HDMI-CEC a TV can only pass arrow keys to the box plugged into it, so the Magic
Remote's pointer does nothing there. magicptr runs on a rooted LG TV and sends the
pointer to the box over the network instead:

- **Hover** follows the TV's cursor and ends when the cursor fades.
- **OK** clicks what you point at; hold it to long-press or drag.
- **Wheel** scrolls, as a real mouse wheel would.
- **Home**: a short press goes to the box's home screen, holding it opens the TV's.

The TV's own cursor is the pointer you see. Nothing is drawn by the box.

## Status

Tested on one setup only: an LG C9 (webOS 4.x) with a Xiaomi TV Box S 3rd gen
(Android TV 14). Other LG models may use different remote key codes or device layouts,
and other boxes may behave differently. Reports of what works and what does not are
welcome.

## Requirements

- An LG TV rooted with the [Homebrew Channel](https://www.webosbrew.org/).
- An Android box on the same network with **network debugging** (ADB over TCP, port
  5555) turned on in its developer options. Boxes that only offer Android's "wireless
  debugging" with a pairing code and a changing port are not supported.

Nothing needs installing on the box by hand, and it does not need root.

## Install

### As a TV app

1. Download the `.ipk` from the latest release and install it on the TV, for example with
   [webOS Dev Manager](https://github.com/webosbrew/dev-manager-desktop).
2. Open **Magic Pointer** on the TV, choose the input the box is on, enter the box's
   address and press **Save and start**.
3. Switch the TV to the box's input. The first time, the box asks whether to allow
   debugging from the TV: tick **Always allow** and accept.

magicptr then starts with the TV. Open the app again to change the settings, see its
status, or turn it off.

### With a script, over SSH

For this the TV needs SSH enabled in the Homebrew Channel. Download and unpack the
`.tar.gz` from the latest release, then run the installer with the TV's address, the
box's address and the number of the HDMI input the box is on:

```sh
./install.sh 192.168.1.20 192.168.1.50 3
```

Then switch to the box's input and accept its debugging prompt, as in step 3 above.

## Troubleshooting

The log is at `/tmp/magicptr.log` on the TV:

- `cannot reach box` — check the box's address and that network debugging is on.
- `box does not know this TV yet` — accept the debugging prompt on the box.
- `expected at least two Magic Remote input devices` — this TV exposes the remote
  differently; please open an issue with the output of `cat /proc/bus/input/devices`.

magicptr only holds the remote while the box's input is in front, and restarts itself,
letting go of the remote, if it ever stalls. Should the remote stop responding on the
box's input anyway, switch input or restart the TV.

## Uninstall

In the app, press **Stop and turn off**, then remove the app. After a script install:

```sh
ssh root@TV 'kill $(cat /tmp/magicptr.pid); rm -r /home/root/magicptr /var/lib/webosbrew/init.d/magicptr'
```

On the box, revoke debugging authorisations in the developer options and delete
`/data/local/tmp/magicptr.dex`.

## Security

The TV holds a key (`/home/root/magicptr/adbkey`) that the box trusts for ADB, which is
shell access to the box. Anyone who can log in to the TV can use it, so change the TV's
default root password and consider turning its telnet server off.

## How it works

`magicptr` is one static program on the TV, started at boot by a Homebrew Channel startup
script. The app is only a settings screen that writes that script. While the box's input
is in the foreground the program:

1. takes exclusive hold of the Magic Remote's input device, so the TV does not react to
   clicks itself, and passes everything except the click, the wheel and Home back to the
   TV through the device of another remote slot;
2. connects to the box with its own small ADB client, copies a helper there
   (`helper/MagicPtr.java`, about 100 lines) and runs it with `app_process`;
3. sends the helper one line per pointer event, which the helper injects as mouse input
   at that position.

## Building

Needs Rust with the `armv7-unknown-linux-musleabihf` target, a JDK and the Android SDK
(for `android.jar` and `d8`; set `ANDROID_HOME` if it is not in `/opt/android-sdk`).

```sh
rustup target add armv7-unknown-linux-musleabihf
./build.sh      # the program, in target/
./package.sh    # the TV app, in dist/ (needs npx)
```

`tools/` has two scripts that run on the TV, for porting to other models: `evcap.py`
prints what the remote sends, and `fakeremote.py` replays pointer input without a remote.

## Licence

MIT
