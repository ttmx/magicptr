//! Forward the LG Magic Remote pointer to an Android box as mouse hover, clicks and scrolling.
//!
//! Runs as root on the TV. While the box's HDMI input is in the foreground it takes over the
//! remote's evdev node and injects pointer input on the box through a helper it installs and
//! runs there over ADB (see helper/). The TV still gets everything but the click, the wheel
//! and Home, passed on through a sibling remote device, so its cursor keeps working without
//! its own UI reacting.

mod adb;

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use adb::Adb;
use rsa::RsaPrivateKey;

const USAGE: &str = "usage: magicptr BOX_HOST[:PORT] WEBOS_APP_ID [KEY_FILE]
  BOX_HOST[:PORT]  the box's ADB endpoint, e.g. 192.168.1.50 (port 5555 unless given)
  WEBOS_APP_ID     the TV input the box is on, e.g. com.webos.app.hdmi3
  KEY_FILE         where to keep the key the box trusts (default: adbkey next to this program)";

static HELPER_DEX: &[u8] = include_bytes!("../helper/magicptr.dex");
const HELPER_PATH: &str = "/data/local/tmp/magicptr.dex";
/// `wm size` first, so the helper's `ready` is preceded by the box's screen size.
const HELPER_COMMAND: &str = "exec:wm size; CLASSPATH=/data/local/tmp/magicptr.dex exec app_process / MagicPtr";

const RETRY_DELAY: Duration = Duration::from_secs(5);
/// Time constant with which a wheel notch is spread into a smooth scroll.
const WHEEL_GLIDE: f64 = 0.07;
const FRAME: Duration = Duration::from_millis(16);
/// Holding Home this long opens the TV's home instead of the box's.
const HOME_HOLD: Duration = Duration::from_millis(500);

const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const EV_ABS: u16 = 3;
const BTN_LEFT: u16 = 0x110;
const REL_WHEEL: u16 = 8;
const ABS_X: u16 = 0;
const ABS_Y: u16 = 1;
/// Enter, up, left, right, down: these hide the TV's cursor.
const DPAD_KEYS: [u16; 5] = [28, 103, 105, 106, 108];
const LG_HOME: u16 = 0x305;
/// Sent by the TV itself when its cursor fades out.
const LG_CURSOR_HIDDEN: u16 = 0x4af;
const ANDROID_HOME: u32 = 3; // KeyEvent.KEYCODE_HOME

const EVIOCGRAB: u32 = 0x4004_4590;
const EVIOCGABS: u32 = 0x8018_4540;
/// The kernel's input_event as this process sees it: a timeval, then type, code and value.
const EVENT_SIZE: usize = 2 * std::mem::size_of::<usize>() + 8;

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        println!("{} {}", $crate::clock(), format!($($arg)*))
    };
}

pub fn clock() -> String {
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&now, &mut tm);
        format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
    }
}

fn event(kind: u16, code: u16, value: i32) -> [u8; EVENT_SIZE] {
    let mut bytes = [0; EVENT_SIZE];
    bytes[EVENT_SIZE - 8..EVENT_SIZE - 6].copy_from_slice(&kind.to_ne_bytes());
    bytes[EVENT_SIZE - 6..EVENT_SIZE - 4].copy_from_slice(&code.to_ne_bytes());
    bytes[EVENT_SIZE - 4..].copy_from_slice(&value.to_ne_bytes());
    bytes
}

/// The pointer on the box, driven through the helper running there.
struct Pointer {
    address: String,
    key: Arc<RsaPrivateKey>,
    adb: Option<Adb>,
    wanted: bool,
    /// Set while a background thread is connecting, so a slow box never stalls the remote.
    connecting: Option<Receiver<io::Result<Adb>>>,
    ready: bool,
    retry_at: Instant,
    pressed: bool,
    /// Position in the remote's coordinates, and the extent of those and of the box's screen.
    position: (i32, i32),
    remote_size: (i32, i32),
    box_size: (i32, i32),
}

impl Pointer {
    fn connect(&mut self) {
        self.wanted = true;
        if let Some(result) = self.connecting.as_ref().and_then(|connecting| connecting.try_recv().ok()) {
            self.connecting = None;
            match result {
                Ok(adb) => {
                    self.adb = Some(adb);
                    log!("connected to box");
                }
                Err(e) => {
                    log!("cannot reach box: {e}");
                    self.retry_at = Instant::now() + RETRY_DELAY;
                }
            }
        }
        if self.adb.is_none() && self.connecting.is_none() && Instant::now() >= self.retry_at {
            let (sender, receiver): (Sender<io::Result<Adb>>, _) = mpsc::channel();
            let (address, key) = (self.address.clone(), self.key.clone());
            thread::spawn(move || {
                let connected = Adb::connect(&address, &key).and_then(|mut adb| {
                    adb.push(HELPER_PATH, HELPER_DEX)?;
                    adb.open(HELPER_COMMAND)?;
                    Ok(adb)
                });
                let _ = sender.send(connected);
            });
            self.connecting = Some(receiver);
        }
    }

    fn disconnect(&mut self) {
        self.wanted = false;
        self.connecting = None;
        self.drop_connection("");
    }

    fn drop_connection(&mut self, reason: &str) {
        if self.adb.take().is_some() {
            self.ready = false;
            self.pressed = false;
            log!("disconnected from box{}{reason}", if reason.is_empty() { "" } else { ": " });
        }
    }

    fn guarded<T>(&mut self, result: io::Result<T>) -> Option<T> {
        result
            .map_err(|e| {
                self.drop_connection(&e.to_string());
                self.retry_at = Instant::now() + RETRY_DELAY;
            })
            .ok()
    }

    /// Handles what the box sent, if anything: the screen size from `wm size`, then the helper's `ready`.
    fn pump(&mut self, readable: bool) {
        let Some(adb) = self.adb.as_mut() else { return };
        let pumped = if readable { adb.pump() } else { Ok(Vec::new()) };
        let checked = adb.check();
        for line in self.guarded(pumped.and_then(|lines| checked.map(|_| lines))).unwrap_or_default() {
            let size = line.rsplit(' ').next().and_then(|size| size.split_once('x'));
            if line == "ready" {
                self.ready = true;
            } else if let Some((Ok(width), Ok(height))) = size.map(|(width, height)| (width.parse(), height.parse())) {
                self.box_size = (width, height); // an override size, if any, comes last
            } else {
                log!("box: {line}");
            }
        }
    }

    fn send(&mut self, line: String, replaceable: bool) -> bool {
        if !self.ready {
            return false;
        }
        let Some(adb) = self.adb.as_mut() else { return false };
        let written = adb.write(line, replaceable);
        self.guarded(written).is_some()
    }

    /// The position on the box's screen.
    fn target(&self) -> (i32, i32) {
        let scale = |value: i32, from: i32, to: i32| (value * to / from.max(1)).clamp(0, to - 1);
        (scale(self.position.0, self.remote_size.0, self.box_size.0), scale(self.position.1, self.remote_size.1, self.box_size.1))
    }

    fn point(&mut self) {
        let (x, y) = self.target();
        self.send(format!("{} {x} {y}", if self.pressed { "move" } else { "hover" }), true);
    }

    fn press(&mut self) {
        if !self.pressed {
            let (x, y) = self.target();
            self.pressed = self.send(format!("down {x} {y}"), false);
        }
    }

    fn release(&mut self) {
        if self.pressed {
            let (x, y) = self.target();
            self.send(format!("up {x} {y}"), false);
            self.pressed = false;
        }
    }

    fn leave(&mut self) {
        self.release();
        self.send("leave".into(), false);
    }

    fn scroll(&mut self, notches: f64) {
        let (x, y) = self.target();
        self.send(format!("scroll {x} {y} {notches:.3}"), false);
    }

    fn tap_key(&mut self, code: u32) {
        self.send(format!("key {code}"), false);
    }
}

/// Event nodes of the Magic Remote receivers: the ones with absolute axes and a mouse handler.
fn pointer_devices() -> io::Result<Vec<String>> {
    Ok(fs::read_to_string("/proc/bus/input/devices")?
        .split("\n\n")
        .filter(|block| block.contains("B: ABS=") && block.contains("mouse"))
        .flat_map(|block| block.split_whitespace().filter(|word| word.starts_with("event")))
        .map(|node| format!("/dev/input/{node}"))
        .collect())
}

fn axis_maximum(device: &File, axis: u16) -> i32 {
    let mut info = [0i32; 6];
    unsafe { libc::ioctl(device.as_raw_fd(), (EVIOCGABS + axis as u32) as _, info.as_mut_ptr()) };
    info[2]
}

fn grab(device: &File, wanted: bool) -> bool {
    unsafe { libc::ioctl(device.as_raw_fd(), EVIOCGRAB as _, wanted as libc::c_int) == 0 }
}

/// Follows which apps the TV has in the foreground, one JSON line per change on its stdout.
fn watch_foreground() -> io::Result<Child> {
    let mut command = Command::new("luna-send");
    command
        .args(["-i", "luna://com.webos.applicationManager/getForegroundAppInfo", r#"{"subscribe":true,"extraInfo":true}"#])
        .stdin(Stdio::null())
        .stdout(Stdio::piped());
    // Have it die with us, however we go, so restarts do not pile watchers up.
    unsafe {
        command.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        })
    };
    command.spawn()
}

fn app_ids(line: &str) -> Vec<&str> {
    line.split(r#""appId":""#).skip(1).filter_map(|rest| rest.split('"').next()).collect()
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !(2..=3).contains(&args.len()) {
        eprintln!("{USAGE}");
        std::process::exit(2);
    }
    let address = if args[0].contains(':') { args[0].clone() } else { format!("{}:5555", args[0]) };
    let app = args[1].as_str();
    let key_file = args.get(2).map(PathBuf::from).unwrap_or(std::env::current_exe()?.with_file_name("adbkey"));
    let key = Arc::new(adb::load_or_create_key(&key_file)?);

    // The TV has one such device per remote slot and its compositor reads them all, so the
    // second slot serves as the way back in for the events of the others we take over.
    let mut nodes = pointer_devices()?;
    if nodes.len() < 2 {
        return Err(io::Error::new(io::ErrorKind::NotFound, format!("expected at least two Magic Remote input devices, found {nodes:?}")));
    }
    let mut sink = OpenOptions::new().write(true).open(nodes.remove(1))?;
    let sources = nodes
        .iter()
        .map(|node| OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(node))
        .collect::<io::Result<Vec<File>>>()?;
    let mut grabbed = vec![false; sources.len()];

    let mut pointer = Pointer {
        address,
        key,
        adb: None,
        wanted: false,
        connecting: None,
        ready: false,
        retry_at: Instant::now(),
        pressed: false,
        position: (0, 0),
        remote_size: (axis_maximum(&sources[0], ABS_X), axis_maximum(&sources[0], ABS_Y)),
        box_size: (1920, 1080),
    };

    let mut luna = watch_foreground()?;
    let mut luna_buf = Vec::new();
    let mut active = false;
    let mut wheel_left = 0.0; // notches the wheel turned that are still to be scrolled
    let mut wheel_stepped = Instant::now();
    let mut home_at: Option<Instant> = None; // when Home went down, while undecided between box and TV

    loop {
        if active {
            pointer.connect();
            for (device, held) in sources.iter().zip(&mut grabbed) {
                // A previous instance may still be exiting with the grab; this retries until it is gone.
                *held = *held || grab(device, true);
            }
        }

        let luna_fd = luna.stdout.as_ref().unwrap().as_raw_fd();
        let mut fds: Vec<RawFd> = sources.iter().map(|device| device.as_raw_fd()).collect();
        fds.push(luna_fd);
        fds.extend(pointer.adb.as_ref().map(|adb| adb.as_raw_fd()));
        let mut polled: Vec<libc::pollfd> = fds.iter().map(|&fd| libc::pollfd { fd, events: libc::POLLIN, revents: 0 }).collect();
        let timeout = if wheel_left != 0.0 {
            FRAME
        } else if home_at.is_some() || pointer.connecting.is_some() {
            Duration::from_millis(100)
        } else {
            Duration::from_secs(1)
        };
        unsafe { libc::poll(polled.as_mut_ptr(), polled.len() as _, timeout.as_millis() as _) };
        let readable = |fd: RawFd| polled.iter().any(|p| p.fd == fd && p.revents != 0);
        let now = Instant::now();

        if readable(luna_fd) {
            let mut chunk = [0; 65536];
            let read = luna.stdout.as_mut().unwrap().read(&mut chunk)?;
            if read == 0 {
                luna.wait()?;
                thread::sleep(Duration::from_secs(1));
                luna = watch_foreground()?;
                luna_buf = b"\n".to_vec(); // an empty line: nothing known to be in the foreground
            }
            luna_buf.extend(&chunk[..read]);
            while let Some(end) = luna_buf.iter().position(|&byte| byte == b'\n') {
                let line: Vec<u8> = luna_buf.drain(..=end).collect();
                let now_active = app_ids(&String::from_utf8_lossy(&line)) == [app];
                if now_active != active {
                    active = now_active;
                    log!("box input {} foreground", if active { "in" } else { "left" });
                    (wheel_left, home_at) = (0.0, None);
                    if !active {
                        for (device, held) in sources.iter().zip(&mut grabbed) {
                            *held = *held && !grab(device, false);
                        }
                        pointer.disconnect();
                    }
                }
            }
        }

        let box_readable = pointer.adb.as_ref().is_some_and(|adb| readable(adb.as_raw_fd()));
        pointer.pump(box_readable);

        for mut device in &sources {
            if !readable(device.as_raw_fd()) {
                continue;
            }
            let mut data = [0; EVENT_SIZE * 64];
            let Ok(read) = device.read(&mut data) else { continue };
            let mut passed = Vec::new();
            let mut moved = false;
            for raw in data[..read].chunks_exact(EVENT_SIZE) {
                let kind = u16::from_ne_bytes(raw[EVENT_SIZE - 8..EVENT_SIZE - 6].try_into().unwrap());
                let code = u16::from_ne_bytes(raw[EVENT_SIZE - 6..EVENT_SIZE - 4].try_into().unwrap());
                let value = i32::from_ne_bytes(raw[EVENT_SIZE - 4..].try_into().unwrap());
                if !matches!((kind, code), (EV_KEY, BTN_LEFT) | (EV_KEY, LG_HOME) | (EV_REL, REL_WHEEL)) {
                    passed.extend(raw);
                }
                match (kind, code) {
                    (EV_ABS, ABS_X) => (pointer.position.0, moved) = (value, true),
                    (EV_ABS, ABS_Y) => (pointer.position.1, moved) = (value, true),
                    _ if !active => {}
                    (EV_KEY, BTN_LEFT) => {
                        pointer.point();
                        match value {
                            1 => pointer.press(),
                            0 => {
                                pointer.release();
                                pointer.point();
                            }
                            _ => {}
                        }
                    }
                    (EV_REL, REL_WHEEL) if !pointer.pressed => {
                        if wheel_left == 0.0 {
                            wheel_stepped = now;
                        }
                        wheel_left += value as f64;
                    }
                    // A tap goes to the box; a hold is handed to the TV once it lasts long enough.
                    (EV_KEY, LG_HOME) if value == 1 => home_at = Some(now),
                    (EV_KEY, LG_HOME) if value == 0 && home_at.take().is_some() => pointer.tap_key(ANDROID_HOME),
                    (EV_KEY, code) if value == 1 && (DPAD_KEYS.contains(&code) || code == LG_CURSOR_HIDDEN) => pointer.leave(),
                    _ => {}
                }
            }
            if active && !passed.is_empty() {
                sink.write_all(&passed)?;
            }
            if active && moved {
                pointer.point();
            }
        }

        if home_at.is_some_and(|pressed| now - pressed > HOME_HOLD) {
            home_at = None;
            for value in [1, 0] {
                sink.write_all(&[event(EV_KEY, LG_HOME, value), event(0, 0, 0)].concat())?;
            }
        }
        if wheel_left != 0.0 {
            let elapsed = (now - wheel_stepped).as_secs_f64();
            let step = if wheel_left.abs() < 0.05 { wheel_left } else { wheel_left * (1.0 - (-elapsed / WHEEL_GLIDE).exp()) };
            wheel_stepped = now;
            wheel_left -= step;
            if step != 0.0 {
                pointer.scroll(step);
            }
            if wheel_left == 0.0 {
                pointer.point(); // whatever scrolled under the pointer is now the hovered item
            }
        }
    }
}
