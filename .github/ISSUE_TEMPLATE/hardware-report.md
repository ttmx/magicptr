---
name: Hardware report
about: Tell us how magicptr behaves on your TV and box, working or not
labels: hardware
---

**TV model and webOS version** (Settings → General → About this TV):

**Box model and Android version**:

**What works and what does not** (hover, click, hold/drag, wheel, Home):

**Log** — output of `ssh root@TV cat /tmp/magicptr.log`:

```
```

**The TV's input devices** — output of `ssh root@TV cat /proc/bus/input/devices`:

```
```

If the pointer does nothing at all, also run `tools/evcap.py` on the TV while moving the
remote and pressing OK, the wheel and Home, and paste what it prints.
