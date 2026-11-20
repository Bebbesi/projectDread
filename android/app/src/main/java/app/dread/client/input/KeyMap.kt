package app.dread.client.input

import android.view.KeyEvent

/** Android key codes → Linux evdev key codes (the protocol's key space). */
object KeyMap {
    const val KEY_ESC = 1
    const val KEY_BACKSPACE = 14
    const val KEY_TAB = 15
    const val KEY_ENTER = 28
    const val KEY_LEFTCTRL = 29
    const val KEY_LEFTSHIFT = 42
    const val KEY_LEFTALT = 56
    const val KEY_LEFTMETA = 125
    const val KEY_DELETE = 111
    const val KEY_UP = 103
    const val KEY_LEFT = 105
    const val KEY_RIGHT = 106
    const val KEY_DOWN = 108
    const val KEY_HOME = 102
    const val KEY_END = 107
    const val KEY_PAGEUP = 104
    const val KEY_PAGEDOWN = 109

    private val map: Map<Int, Int> = buildMap {
        val letters = intArrayOf(30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20, 22, 47, 17, 45, 21, 44)
        for (i in 0 until 26) put(KeyEvent.KEYCODE_A + i, letters[i])
        val digits = intArrayOf(11, 2, 3, 4, 5, 6, 7, 8, 9, 10)
        for (i in 0..9) put(KeyEvent.KEYCODE_0 + i, digits[i])
        val f = intArrayOf(59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 87, 88)
        for (i in 0 until 12) put(KeyEvent.KEYCODE_F1 + i, f[i])
        put(KeyEvent.KEYCODE_ESCAPE, KEY_ESC)
        put(KeyEvent.KEYCODE_DEL, KEY_BACKSPACE)
        put(KeyEvent.KEYCODE_FORWARD_DEL, KEY_DELETE)
        put(KeyEvent.KEYCODE_TAB, KEY_TAB)
        put(KeyEvent.KEYCODE_ENTER, KEY_ENTER)
        put(KeyEvent.KEYCODE_NUMPAD_ENTER, 96)
        put(KeyEvent.KEYCODE_SPACE, 57)
        put(KeyEvent.KEYCODE_MINUS, 12)
        put(KeyEvent.KEYCODE_EQUALS, 13)
        put(KeyEvent.KEYCODE_LEFT_BRACKET, 26)
        put(KeyEvent.KEYCODE_RIGHT_BRACKET, 27)
        put(KeyEvent.KEYCODE_BACKSLASH, 43)
        put(KeyEvent.KEYCODE_SEMICOLON, 39)
        put(KeyEvent.KEYCODE_APOSTROPHE, 40)
        put(KeyEvent.KEYCODE_GRAVE, 41)
        put(KeyEvent.KEYCODE_COMMA, 51)
        put(KeyEvent.KEYCODE_PERIOD, 52)
        put(KeyEvent.KEYCODE_SLASH, 53)
        put(KeyEvent.KEYCODE_CTRL_LEFT, KEY_LEFTCTRL)
        put(KeyEvent.KEYCODE_CTRL_RIGHT, 97)
        put(KeyEvent.KEYCODE_SHIFT_LEFT, KEY_LEFTSHIFT)
        put(KeyEvent.KEYCODE_SHIFT_RIGHT, 54)
        put(KeyEvent.KEYCODE_ALT_LEFT, KEY_LEFTALT)
        put(KeyEvent.KEYCODE_ALT_RIGHT, 100)
        put(KeyEvent.KEYCODE_META_LEFT, KEY_LEFTMETA)
        put(KeyEvent.KEYCODE_META_RIGHT, 126)
        put(KeyEvent.KEYCODE_CAPS_LOCK, 58)
        put(KeyEvent.KEYCODE_NUM_LOCK, 69)
        put(KeyEvent.KEYCODE_SCROLL_LOCK, 70)
        put(KeyEvent.KEYCODE_SYSRQ, 99)
        put(KeyEvent.KEYCODE_BREAK, 119)
        put(KeyEvent.KEYCODE_INSERT, 110)
        put(KeyEvent.KEYCODE_MOVE_HOME, KEY_HOME)
        put(KeyEvent.KEYCODE_MOVE_END, KEY_END)
        put(KeyEvent.KEYCODE_PAGE_UP, KEY_PAGEUP)
        put(KeyEvent.KEYCODE_PAGE_DOWN, KEY_PAGEDOWN)
        put(KeyEvent.KEYCODE_DPAD_UP, KEY_UP)
        put(KeyEvent.KEYCODE_DPAD_DOWN, KEY_DOWN)
        put(KeyEvent.KEYCODE_DPAD_LEFT, KEY_LEFT)
        put(KeyEvent.KEYCODE_DPAD_RIGHT, KEY_RIGHT)
        put(KeyEvent.KEYCODE_MENU, 127)
        put(KeyEvent.KEYCODE_VOLUME_MUTE, 113)
        put(KeyEvent.KEYCODE_MEDIA_PLAY_PAUSE, 164)
        put(KeyEvent.KEYCODE_MEDIA_NEXT, 163)
        put(KeyEvent.KEYCODE_MEDIA_PREVIOUS, 165)
        val np = intArrayOf(82, 79, 80, 81, 75, 76, 77, 71, 72, 73)
        for (i in 0..9) put(KeyEvent.KEYCODE_NUMPAD_0 + i, np[i])
        put(KeyEvent.KEYCODE_NUMPAD_DIVIDE, 98)
        put(KeyEvent.KEYCODE_NUMPAD_MULTIPLY, 55)
        put(KeyEvent.KEYCODE_NUMPAD_SUBTRACT, 74)
        put(KeyEvent.KEYCODE_NUMPAD_ADD, 78)
        put(KeyEvent.KEYCODE_NUMPAD_DOT, 83)
    }

    fun evdev(androidKeyCode: Int): Int? = map[androidKeyCode]
}
