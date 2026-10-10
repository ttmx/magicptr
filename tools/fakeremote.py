# Replay Magic Remote input on the TV (python 2.7), to test magicptr without the remote.
# Writes to /dev/input/event3, the first remote slot on an LG C9, unless DEVICE names another.
# Usage: [DEVICE=/dev/input/eventN] fakeremote.py move X Y | click X Y | wheel X Y TICKS
import os, struct, sys, time
fd = os.open(os.environ.get("DEVICE", "/dev/input/event3"), os.O_WRONLY)
def ev(t, c, v):
    os.write(fd, struct.pack("IIHHi", 0, 0, t, c, v))
def syn():
    ev(0, 0, 0)
what, x, y = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
for i in range(10, 0, -1):      # glide in from the right, as a hand would
    ev(3, 0, x + i * 20); ev(3, 1, y); syn()
    time.sleep(0.02)
ev(3, 0, x); ev(3, 1, y); syn()
time.sleep(0.2)
if what == "click":
    ev(1, 0x110, 1); syn()
    time.sleep(0.12)
    ev(1, 0x110, 0); syn()
elif what == "wheel":
    n = int(sys.argv[4])
    for i in range(abs(n)):
        ev(2, 8, 1 if n > 0 else -1); syn()
        time.sleep(0.05)
