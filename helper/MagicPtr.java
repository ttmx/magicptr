import android.os.SystemClock;
import android.view.InputDevice;
import android.view.InputEvent;
import android.view.KeyCharacterMap;
import android.view.KeyEvent;
import android.view.MotionEvent;

import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.lang.reflect.Method;

/**
 * Injects pointer input on the box, one command per line on stdin:
 *
 *   hover X Y      mouse pointer hovering at X, Y
 *   leave          the pointer is gone
 *   down X Y       primary button down; then `move X Y` and `up X Y`
 *   scroll X Y V   mouse wheel at X, Y: V notches, positive is up
 *   key CODE       press and release an Android key code
 *
 * Everything is a mouse, clicks included: a finger tap would put the box in touch mode, which
 * drops the focus highlight for the length of the click and brings it back with a second sound.
 *
 * Run by the shell user with app_process, which is allowed to inject input.
 */
public final class MagicPtr {
    private static Object inputManager;
    private static Method injectInputEvent;
    private static long downTime;
    private static boolean hovering;
    private static float hoverX, hoverY;

    public static void main(String[] args) throws Exception {
        Class<?> manager;
        try {
            manager = Class.forName("android.hardware.input.InputManagerGlobal");
        } catch (ClassNotFoundException e) {
            manager = Class.forName("android.hardware.input.InputManager");
        }
        inputManager = manager.getMethod("getInstance").invoke(null);
        injectInputEvent = manager.getMethod("injectInputEvent", InputEvent.class, int.class);
        System.out.println("ready");

        BufferedReader in = new BufferedReader(new InputStreamReader(System.in));
        for (String line; (line = in.readLine()) != null; ) {
            String[] word = line.split(" ");
            float x = word.length > 2 ? Float.parseFloat(word[1]) : 0;
            float y = word.length > 2 ? Float.parseFloat(word[2]) : 0;
            switch (word[0]) {
                case "hover":
                    hovering = true;
                    hoverX = x;
                    hoverY = y;
                    hover(MotionEvent.ACTION_HOVER_MOVE, x, y, 0);
                    break;
                case "leave":
                    leave();
                    break;
                case "down":
                    leave();
                    downTime = SystemClock.uptimeMillis();
                    motion(downTime, MotionEvent.ACTION_DOWN, x, y, 0, MotionEvent.BUTTON_PRIMARY);
                    break;
                case "move":
                    motion(downTime, MotionEvent.ACTION_MOVE, x, y, 0, MotionEvent.BUTTON_PRIMARY);
                    break;
                case "up":
                    motion(downTime, MotionEvent.ACTION_UP, x, y, 0, 0);
                    break;
                case "scroll":
                    hover(MotionEvent.ACTION_SCROLL, x, y, Float.parseFloat(word[3]));
                    break;
                case "key":
                    key(KeyEvent.ACTION_DOWN, Integer.parseInt(word[1]));
                    key(KeyEvent.ACTION_UP, Integer.parseInt(word[1]));
                    break;
            }
        }
    }

    private static void leave() throws Exception {
        if (hovering) {
            hover(MotionEvent.ACTION_HOVER_EXIT, hoverX, hoverY, 0);
            hovering = false;
        }
    }

    /** A motion with no button held. */
    private static void hover(int action, float x, float y, float vscroll) throws Exception {
        motion(SystemClock.uptimeMillis(), action, x, y, vscroll, 0);
    }

    private static void motion(long downTime, int action, float x, float y, float vscroll, int buttons)
            throws Exception {
        MotionEvent.PointerProperties[] properties = {new MotionEvent.PointerProperties()};
        properties[0].id = 0;
        properties[0].toolType = MotionEvent.TOOL_TYPE_MOUSE;
        MotionEvent.PointerCoords[] coords = {new MotionEvent.PointerCoords()};
        coords[0].x = x;
        coords[0].y = y;
        coords[0].pressure = buttons != 0 ? 1 : 0;
        coords[0].size = 1;
        coords[0].setAxisValue(MotionEvent.AXIS_VSCROLL, vscroll);
        inject(MotionEvent.obtain(downTime, SystemClock.uptimeMillis(), action, 1, properties, coords, 0, buttons,
                1, 1, KeyCharacterMap.VIRTUAL_KEYBOARD, 0, InputDevice.SOURCE_MOUSE, 0));
    }

    private static void key(int action, int code) throws Exception {
        long now = SystemClock.uptimeMillis();
        inject(new KeyEvent(now, now, action, code, 0, 0, KeyCharacterMap.VIRTUAL_KEYBOARD, 0, 0,
                InputDevice.SOURCE_KEYBOARD));
    }

    private static void inject(InputEvent event) throws Exception {
        injectInputEvent.invoke(inputManager, event, 0);
    }
}
