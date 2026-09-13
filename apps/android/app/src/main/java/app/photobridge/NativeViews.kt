package app.photobridge

import android.content.Context
import androidx.appcompat.app.AppCompatActivity
import com.google.android.material.appbar.MaterialToolbar
import android.util.TypedValue
import android.view.View
import android.widget.*
import com.google.android.material.button.MaterialButton
import com.google.android.material.card.MaterialCardView

internal fun Context.dp(value: Int) = (value * resources.displayMetrics.density).toInt()
internal fun Context.themeColor(attribute: Int): Int {
    val value = TypedValue()
    theme.resolveAttribute(attribute, value, true)
    return if (value.resourceId != 0) getColor(value.resourceId) else value.data
}
internal fun Context.secondaryColor() = themeColor(android.R.attr.textColorSecondary)
internal fun Context.accentColor() = themeColor(com.google.android.material.R.attr.colorPrimary)
internal fun Context.label(parent: LinearLayout, value: String, size: Int, color: Int = themeColor(android.R.attr.textColorPrimary)) = TextView(this).also {
    it.text = value; it.textSize = size.toFloat(); it.setTextColor(color)
    it.setPadding(0, dp(6), 0, dp(8)); parent.addView(it)
}
internal fun Context.section(parent: LinearLayout, title: Int) {
    label(parent, getString(title), 14, secondaryColor()).setPadding(dp(4), dp(20), 0, dp(8))
}
internal fun Context.action(parent: LinearLayout, title: Int, primary: Boolean = false, action: () -> Unit): MaterialButton {
    val style = if (primary) com.google.android.material.R.attr.materialButtonStyle else com.google.android.material.R.attr.materialButtonOutlinedStyle
    return MaterialButton(this, null, style).also {
        it.setText(title); it.isAllCaps = false; it.minimumHeight = dp(52)
        it.setOnClickListener { action() }
        parent.addView(it, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(4) })
    }
}
internal fun Context.card(parent: LinearLayout, content: (LinearLayout) -> Unit) {
    val body = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setPadding(dp(18), dp(12), dp(18), dp(16)) }
    val card = MaterialCardView(this).apply { radius = dp(18).toFloat(); cardElevation = 0f; addView(body) }
    parent.addView(card, LinearLayout.LayoutParams(-1, -2).apply { bottomMargin = dp(12) })
    content(body)
}
internal fun Context.scrollPage(content: (LinearLayout) -> Unit): View {
    val panel = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setPadding(dp(20), dp(12), dp(20), dp(24)) }
    content(panel)
    return ScrollView(this).apply { isFillViewport = true; addView(panel) }
}

internal fun AppCompatActivity.nativeDetailPage(titleId: Int, content: (LinearLayout) -> Unit) {
    val root = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
    val toolbar = MaterialToolbar(this).apply {
        setTitle(titleId)
        setNavigationIcon(R.drawable.ic_back)
        setNavigationContentDescription(R.string.nav_back)
        setNavigationOnClickListener { finish() }
    }
    root.addView(toolbar, LinearLayout.LayoutParams(-1, dp(64)))
    root.addView(scrollPage(content), LinearLayout.LayoutParams(-1, 0, 1f))
    setContentView(root)
}
