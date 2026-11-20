package app.dread.client.ui

import android.app.AlertDialog
import android.content.Context
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.StateListDrawable
import android.text.InputType
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.TextView

/** DREAD's flat monochrome palette and view helpers (no XML layouts needed). */
object Ui {
    val BG = Color.parseColor("#101010")
    val BG2 = Color.parseColor("#181818")
    val PANEL = Color.parseColor("#222222")
    val PANEL_HI = Color.parseColor("#2C2C2C")
    val BORDER = Color.parseColor("#353535")
    val MID = Color.parseColor("#6A6A6A")
    val TEXT2 = Color.parseColor("#A0A0A0")
    val TEXT = Color.WHITE
    val LIGHT = Color.parseColor("#D8D8D8")
    val MONO: Typeface = Typeface.MONOSPACE

    fun Context.dp(v: Float): Int = TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, v, resources.displayMetrics).toInt()
    fun Context.dp(v: Int): Int = dp(v.toFloat())

    fun box(fill: Int, stroke: Int = BORDER, radius: Float = 0f, ctx: Context): GradientDrawable =
        GradientDrawable().apply {
            setColor(fill)
            setStroke(ctx.dp(1), stroke)
            cornerRadius = radius
        }

    fun pressable(ctx: Context, fill: Int, pressed: Int, stroke: Int = BORDER): StateListDrawable =
        StateListDrawable().apply {
            addState(intArrayOf(android.R.attr.state_pressed), box(pressed, stroke, ctx.dp(2f).toFloat(), ctx))
            addState(intArrayOf(), box(fill, stroke, ctx.dp(2f).toFloat(), ctx))
        }

    fun text(ctx: Context, s: CharSequence, size: Float = 14f, color: Int = TEXT, bold: Boolean = false, mono: Boolean = false) =
        TextView(ctx).apply {
            text = s
            setTextColor(color)
            setTextSize(TypedValue.COMPLEX_UNIT_SP, size)
            if (mono) typeface = MONO
            if (bold) setTypeface(typeface, Typeface.BOLD)
        }

    fun caption(ctx: Context, s: CharSequence) = text(ctx, s, 12f, TEXT2)

    fun sectionTitle(ctx: Context, s: String) = text(ctx, s.uppercase(), 11f, TEXT2, bold = true).apply {
        letterSpacing = 0.08f
        setPadding(0, ctx.dp(18), 0, ctx.dp(8))
    }

    /** Flat button; [primary] = light fill with dark text. */
    fun button(ctx: Context, label: String, primary: Boolean = false, onClick: () -> Unit) = TextView(ctx).apply {
        text = label
        gravity = Gravity.CENTER
        setTextSize(TypedValue.COMPLEX_UNIT_SP, 14f)
        setTextColor(if (primary) BG else TEXT)
        if (primary) setTypeface(typeface, Typeface.BOLD)
        background = if (primary) pressable(ctx, LIGHT, Color.WHITE, LIGHT) else pressable(ctx, PANEL, PANEL_HI)
        setPadding(ctx.dp(16), ctx.dp(11), ctx.dp(16), ctx.dp(11))
        minHeight = ctx.dp(44)
        isClickable = true
        setOnClickListener { onClick() }
    }

    fun input(ctx: Context, hint: String, value: String = "", numeric: Boolean = false) = EditText(ctx).apply {
        setText(value)
        this.hint = hint
        setHintTextColor(MID)
        setTextColor(TEXT)
        setTextSize(TypedValue.COMPLEX_UNIT_SP, 15f)
        typeface = MONO
        background = box(BG, BORDER, ctx.dp(2f).toFloat(), ctx)
        setPadding(ctx.dp(12), ctx.dp(10), ctx.dp(12), ctx.dp(10))
        isSingleLine = true
        inputType = if (numeric) InputType.TYPE_CLASS_NUMBER else InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI
    }

    fun vertical(ctx: Context, pad: Int = 0) = LinearLayout(ctx).apply {
        orientation = LinearLayout.VERTICAL
        setPadding(pad, pad, pad, pad)
    }

    fun horizontal(ctx: Context) = LinearLayout(ctx).apply {
        orientation = LinearLayout.HORIZONTAL
        gravity = Gravity.CENTER_VERTICAL
    }

    fun panel(ctx: Context) = vertical(ctx, ctx.dp(14)).apply { background = box(PANEL, BORDER, 0f, ctx) }

    fun icon(ctx: Context, res: Int, size: Int = 22) = ImageView(ctx).apply {
        setImageResource(res)
        layoutParams = LinearLayout.LayoutParams(ctx.dp(size), ctx.dp(size))
    }

    fun lp(w: Int = ViewGroup.LayoutParams.MATCH_PARENT, h: Int = ViewGroup.LayoutParams.WRAP_CONTENT, weight: Float = 0f, top: Int = 0, end: Int = 0) =
        LinearLayout.LayoutParams(w, h, weight).apply { topMargin = top; marginEnd = end }

    fun flp(w: Int, h: Int, gravity: Int) = FrameLayout.LayoutParams(w, h, gravity)

    /** A clickable settings row: title, current value, opens [onClick]. */
    fun row(ctx: Context, title: String, value: String, onClick: () -> Unit): Pair<View, TextView> {
        val v = horizontal(ctx).apply {
            setPadding(0, ctx.dp(12), 0, ctx.dp(12))
            background = pressable(ctx, Color.TRANSPARENT, PANEL_HI, Color.TRANSPARENT)
            isClickable = true
            setOnClickListener { onClick() }
        }
        val t = text(ctx, title, 15f)
        val valueView = text(ctx, value, 14f, TEXT2)
        v.addView(t, lp(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
        v.addView(valueView)
        return v to valueView
    }

    fun <T> choose(ctx: Context, title: String, options: List<T>, label: (T) -> String, current: T, onPick: (T) -> Unit) {
        AlertDialog.Builder(ctx)
            .setTitle(title)
            .setSingleChoiceItems(options.map(label).toTypedArray(), options.indexOf(current).coerceAtLeast(0)) { d, i ->
                onPick(options[i]); d.dismiss()
            }
            .setNegativeButton("Cancel", null)
            .show()
    }

    fun divider(ctx: Context) = View(ctx).apply {
        setBackgroundColor(BORDER)
        layoutParams = LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ctx.dp(1))
    }
}
