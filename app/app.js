// Settings screen for magicptr. Everything privileged goes through the Homebrew Channel's
// root exec service: this page only writes the boot script that starts the program.

var APP_DIR = '/media/developer/apps/usr/palm/applications/io.github.ttmx.magicptr';
var BOOT_SCRIPT = '/var/lib/webosbrew/init.d/magicptr';
var DATA_DIR = '/home/root/magicptr';

var chosenInput = null;

function call(uri, params, done) {
    var bridge = new (window.WebOSServiceBridge || window.PalmServiceBridge)();
    bridge.onservicecallback = function (message) { done(JSON.parse(message)); };
    bridge.call(uri, JSON.stringify(params));
}

function run(command, done) {
    call('luna://org.webosbrew.hbchannel.service/exec', { command: command }, function (reply) {
        done(reply.stdoutString || '', reply);
    });
}

function showInputs(devices) {
    var list = document.getElementById('inputs');
    list.innerHTML = '';
    devices.forEach(function (device) {
        var names = (device.subList || []).map(function (sub) { return sub.osdName; }).filter(Boolean);
        var button = document.createElement('button');
        button.textContent = device.label + (names.length ? ' (' + names[0] + ')' : '');
        button.onclick = function () { choose(device.appId); };
        button.dataset.app = device.appId;
        list.appendChild(button);
    });
    choose(chosenInput);
}

function choose(appId) {
    chosenInput = appId;
    [].forEach.call(document.querySelectorAll('#inputs button'), function (button) {
        button.className = button.dataset.app === appId ? 'chosen' : '';
    });
}

function refresh() {
    var command = 'kill -0 $(cat /tmp/magicptr.pid 2>/dev/null) 2>/dev/null && echo running; ' +
        'tail -n 8 /tmp/magicptr.log 2>/dev/null';
    run(command, function (output) {
        var running = output.indexOf('running\n') === 0;
        document.getElementById('state').textContent = running ? 'Running' : 'Not running';
        document.getElementById('log').textContent = output.replace(/^running\n/, '');
    });
}

function start() {
    var address = document.getElementById('address').value.trim();
    // Both values end up in a root shell script, so nothing but the expected characters gets through.
    if (!/^[A-Za-z0-9.:-]+$/.test(address) || !/^[A-Za-z0-9.]+$/.test(chosenInput || '')) {
        document.getElementById('state').textContent = 'Choose an input and enter the box\'s address first';
        return;
    }
    var script = [
        '#!/bin/sh',
        '# Starts magicptr at boot; written by the Magic Pointer app.',
        '[ -x ' + APP_DIR + '/bin/magicptr ] || exit 0',
        'kill $(cat /tmp/magicptr.pid 2>/dev/null) 2>/dev/null',
        APP_DIR + '/bin/magicptr ' + address + ' ' + chosenInput + ' ' + DATA_DIR + '/adbkey >/tmp/magicptr.log 2>&1 </dev/null &',
        'echo $! > /tmp/magicptr.pid'
    ].join('\n');
    var command = 'mkdir -p ' + DATA_DIR + ' && chmod 700 ' + DATA_DIR + ' && chmod +x ' + APP_DIR + '/bin/magicptr && ' +
        "cat > " + BOOT_SCRIPT + " <<'EOF'\n" + script + "\nEOF\n" +
        'chmod +x ' + BOOT_SCRIPT + ' && ' + BOOT_SCRIPT;
    run(command, refresh);
}

function stop() {
    run('kill $(cat /tmp/magicptr.pid 2>/dev/null) 2>/dev/null; rm -f ' + BOOT_SCRIPT, refresh);
}

// Arrow keys move between the controls, for remotes used without the pointer. They stop on the
// address field's row rather than the field, which would bring up the on-screen keyboard; OK
// on the row goes into the field.
document.addEventListener('keydown', function (event) {
    var row = document.getElementById('address-row');
    var field = document.getElementById('address');
    if (event.keyCode === 13 && document.activeElement === row) {
        field.focus();
        return;
    }
    var step = { 38: -1, 40: 1 }[event.keyCode];
    if (!step) return;
    var controls = [].slice.call(document.querySelectorAll('button, #address-row'));
    var current = document.activeElement === field ? row : document.activeElement;
    var next = controls[controls.indexOf(current) + step];
    if (next) next.focus();
    event.preventDefault();
});

document.getElementById('start').onclick = start;
document.getElementById('stop').onclick = stop;

// Start from the current setup, if there is one.
run('grep -o "magicptr [^ ]* com[.][^ ]*" ' + BOOT_SCRIPT + ' 2>/dev/null', function (output) {
    var current = output.trim().split(' ');
    if (current.length === 3) {
        document.getElementById('address').value = current[1];
        chosenInput = current[2];
    }
    call('luna://com.webos.service.eim/getAllInputStatus', {}, function (reply) {
        showInputs((reply.devices || []).filter(function (device) { return /^HDMI/.test(device.id); }));
        var first = document.querySelector('#inputs button.chosen') || document.querySelector('#inputs button');
        if (first) first.focus();
    });
});
refresh();
setInterval(refresh, 2000);
