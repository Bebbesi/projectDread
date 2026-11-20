package app.dread.client.input

import android.content.Context
import android.text.InputType
import android.view.KeyEvent
import android.view.View
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager
import app.dread.client.net.Protocol

/**
 * Invisible view that owns the Android soft keyboard and forwards what is
 * typed to the computer: committed text as Unicode, composing text (word
 * suggestions, CJK input) as live edits, editing keys as key events.
 */
class KeyboardSink(ctx: Context, private val send: (ByteArray) -> Unit) : View(ctx) {
    var suggestions = false
    /** Modifiers latched from the special-keys bar; applied to the next key. */
    var latchedModifiers: () -> List<Int> = { emptyList() }
    var clearLatched: () -> Unit = {}

    private var composing = ""

    init {
        isFocusable = true
        isFocusableInTouchMode = true
    }

    override fun onCheckIsTextEditor() = true

    override fun onCreateInputConnection(outAttrs: EditorInfo): InputConnection {
        outAttrs.inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_MULTI_LINE or
            (if (suggestions) InputType.TYPE_TEXT_FLAG_AUTO_CORRECT else InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS or InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD)
        outAttrs.imeOptions = EditorInfo.IME_FLAG_NO_FULLSCREEN or EditorInfo.IME_FLAG_NO_EXTRACT_UI or EditorInfo.IME_ACTION_NONE
        composing = ""
        return Conn()
    }

    fun show() {
        requestFocus()
        val imm = context.getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager
        imm.restartInput(this)
        imm.showSoftInput(this, InputMethodManager.SHOW_IMPLICIT)
    }

    fun hide() {
        val imm = context.getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager
        imm.hideSoftInputFromWindow(windowToken, 0)
        composing = ""
    }

    fun tapKey(code: Int) {
        val mods = latchedModifiers()
        mods.forEach { send(Protocol.key(it, true)) }
        send(Protocol.key(code, true))
        send(Protocol.key(code, false))
        mods.reversed().forEach { send(Protocol.key(it, false)) }
        if (mods.isNotEmpty()) clearLatched()
    }

    private fun typeText(t: String) {
        if (t.isEmpty()) return
        val mods = latchedModifiers()
        if (mods.isNotEmpty() && t.length == 1) {
            // Ctrl+C etc.: send as key chord when the character has a key
            val kc = KeyEvent.keyCodeFromString("KEYCODE_" + t.uppercase())
            val ev = KeyMap.evdev(kc)
            if (ev != null) {
                tapKey(ev)
                return
            }
        }
        for (part in t.split('\n').withIndex()) {
            if (part.index > 0) tapKey(KeyMap.KEY_ENTER)
            if (part.value.isNotEmpty()) send(Protocol.text(part.value))
        }
    }

    /** Replaces the previously sent composing text with [new]. */
    private fun replaceComposing(new: String) {
        val old = composing
        var p = 0
        while (p < old.length && p < new.length && old[p] == new[p]) p++
        repeat(old.length - p) { tapKey(KeyMap.KEY_BACKSPACE) }
        typeText(new.substring(p))
        composing = new
    }

    private inner class Conn : BaseInputConnection(this@KeyboardSink, false) {
        override fun commitText(text: CharSequence, newCursorPosition: Int): Boolean {
            replaceComposing(text.toString())
            composing = ""
            return true
        }

        override fun setComposingText(text: CharSequence, newCursorPosition: Int): Boolean {
            replaceComposing(text.toString())
            return true
        }

        override fun finishComposingText(): Boolean {
            composing = ""
            return true
        }

        override fun deleteSurroundingText(beforeLength: Int, afterLength: Int): Boolean {
            repeat(beforeLength.coerceAtMost(64)) { tapKey(KeyMap.KEY_BACKSPACE) }
            repeat(afterLength.coerceAtMost(64)) { tapKey(KeyMap.KEY_DELETE) }
            return true
        }

        override fun sendKeyEvent(event: KeyEvent): Boolean {
            val code = KeyMap.evdev(event.keyCode) ?: return true
            if (event.action == KeyEvent.ACTION_DOWN) {
                if (composing.isNotEmpty() && event.keyCode == KeyEvent.KEYCODE_DEL) {
                    composing = composing.dropLast(1)
                }
                tapKey(code)
            }
            return true
        }

        override fun performEditorAction(actionCode: Int): Boolean {
            tapKey(KeyMap.KEY_ENTER)
            return true
        }
    }
}
