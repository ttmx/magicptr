#!/bin/sh
# Install magicptr on a rooted LG TV and start it.
# Usage: install.sh TV_HOST BOX_HOST[:PORT] HDMI_INPUT_NUMBER
#   e.g. install.sh 192.168.1.20 192.168.1.50 3
set -eu
[ $# -eq 3 ] || { sed -n '2,4s/^# //p' "$0" >&2; exit 2; }
TV=root@$1
BOX=$2
APP=com.webos.app.hdmi$3
DIR=/home/root/magicptr
INIT=/var/lib/webosbrew/init.d/magicptr

# A release has the program next to this script; a checkout has it where cargo built it.
cd "$(dirname "$0")"
BIN=magicptr
[ -f $BIN ] || BIN=target/armv7-unknown-linux-musleabihf/release/magicptr
[ -f $BIN ] || { echo "no magicptr program found; download a release or run ./build.sh" >&2; exit 1; }

# One connection for everything: the TV's sshd has no scp on its PATH.
ssh "$TV" "
    mkdir -p $DIR && chmod 700 $DIR &&
    cat > $DIR/magicptr.new && chmod +x $DIR/magicptr.new && mv $DIR/magicptr.new $DIR/magicptr &&
    cat > $INIT <<EOF && chmod +x $INIT && $INIT
#!/bin/sh
# Started at boot by the Homebrew Channel; safe to re-run.
kill \\\$(cat /tmp/magicptr.pid 2>/dev/null) 2>/dev/null
$DIR/magicptr $BOX $APP >/tmp/magicptr.log 2>&1 </dev/null &
echo \\\$! > /tmp/magicptr.pid
EOF
" < $BIN
echo "Installed. If the box asks whether to allow debugging, tick 'Always allow' and accept."
echo "Log: ssh $TV cat /tmp/magicptr.log"
