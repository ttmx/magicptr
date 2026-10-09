# Dump Magic Remote evdev activity on the TV (python 2.7). Usage: evcap.py SECONDS
import os, select, struct, time, sys
fmt = "llHHi" if struct.calcsize("P") == 8 else "IIHHi"
sz = struct.calcsize(fmt)
fds = {}
for n in (0, 1, 3, 4, 5):
    fds[os.open("/dev/input/event%d" % n, os.O_RDONLY | os.O_NONBLOCK)] = n
end = time.time() + float(sys.argv[1])
absn = {}
while time.time() < end:
    r, _, _ = select.select(list(fds), [], [], 0.5)
    for fd in r:
        try:
            data = os.read(fd, sz * 64)
        except OSError:
            continue
        for i in range(0, len(data) - sz + 1, sz):
            s, us, t, c, v = struct.unpack(fmt, data[i:i + sz])
            if t == 0:
                continue
            if t == 3:
                k = (fds[fd], c)
                absn[k] = (absn.get(k, (0, 0))[0] + 1, v)
                continue
            print("ev%d %.3f type=%d code=%d(0x%x) val=%d abs=%r" % (fds[fd], s + us / 1e6, t, c, c, v, absn))
            sys.stdout.flush()
print("abs (dev,axis)->(count,last): %r ptr=%d" % (absn, struct.calcsize("P") * 8))
