package dev.timeslay.day.widget

import android.content.Context
import android.content.res.Configuration
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import android.graphics.Typeface
import android.text.TextPaint
import android.text.TextUtils
import java.io.File
import kotlin.math.floor
import kotlin.math.max
import kotlin.math.min

/** Keep in sync with `day_widget_hour_height` in day_widget.xml. */
const val HOUR_HEIGHT_DP = 60f

/** Keep in sync with `day_widget_all_day_height` in day_widget.xml. */
const val ALL_DAY_ROW_DP = 30f

private const val GUTTER_DP = 34f
private const val END_PADDING_DP = 8f
private const val ITEM_GAP_DP = 2f
private const val RADIUS_DP = 6f

/** The shortest a travel box is drawn, so its words fit in it. */
private const val TRAVEL_MIN_DP = 16f

/** Below this a column cannot show a useful word, so the rest collapse into "+N". */
private const val MIN_COLUMN_DP = 58f

class WidgetColors(
    val surface: Int,
    val text: Int,
    val muted: Int,
    val line: Int,
    val now: Int,
    val accent: Int,
    val onAccent: Int,
    val overflow: Int,
) {
    companion object {
        fun of(context: Context): WidgetColors {
            val preference = try {
                // `Theme::path` in dioxus/src/day.rs.
                File(context.filesDir, "config/timeslay/dioxus-theme").readText().trim()
            } catch (_: Exception) {
                ""
            }
            val night = context.resources.configuration.uiMode and
                Configuration.UI_MODE_NIGHT_MASK == Configuration.UI_MODE_NIGHT_YES
            // The high contrast choices share these prefixes.
            val dark = when {
                preference.startsWith("light") -> false
                preference.startsWith("dark") -> true
                else -> night
            }
            return if (dark) {
                WidgetColors(
                    surface = 0xFF15171E.toInt(),
                    text = 0xFFE8EAF0.toInt(),
                    muted = 0xFF8D93A3.toInt(),
                    line = 0xFF2A2D37.toInt(),
                    now = 0xFFFF5A52.toInt(),
                    accent = 0xFF7C9CF5.toInt(),
                    onAccent = 0xFF10131A.toInt(),
                    overflow = 0xFF343845.toInt(),
                )
            } else {
                WidgetColors(
                    surface = 0xFFFCFCFD.toInt(),
                    text = 0xFF22252D.toInt(),
                    muted = 0xFF6F7583.toInt(),
                    line = 0xFFE8E9ED.toInt(),
                    now = 0xFFE5392F.toInt(),
                    accent = 0xFF4F7BEF.toInt(),
                    onAccent = 0xFFFFFFFF.toInt(),
                    overflow = 0xFFE2E4EA.toInt(),
                )
            }
        }
    }
}

/**
 * Draws the day. A widget's list can only hold whole views of a fixed set of
 * types, with no way to place one at a time of day, so the timeline is drawn as
 * one tall picture and handed to the list an hour at a time. Each row only
 * differs in where the canvas is translated, which keeps the seams invisible.
 */
class DayRenderer(
    private val data: WidgetDay,
    private val colors: WidgetColors,
    private val widthPx: Int,
    private val density: Float,
    private val fontScale: Float,
) {
    private val hourHeight = HOUR_HEIGHT_DP * density
    private val gutter = dp(GUTTER_DP)
    private val laneStart = gutter + dp(4f)
    private val laneWidth = widthPx - laneStart - dp(END_PADDING_DP)
    private val maxColumns = max(1, floor(laneWidth / dp(MIN_COLUMN_DP)).toInt())
    private val fill = Paint(Paint.ANTI_ALIAS_FLAG)
    private val stroke = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.STROKE }
    private val text = TextPaint(Paint.ANTI_ALIAS_FLAG)
    private val bold = Typeface.create("sans-serif-medium", Typeface.NORMAL)
    private val regular = Typeface.create("sans-serif", Typeface.NORMAL)

    private fun dp(value: Float) = value * density
    private fun sp(value: Float) = value * density * fontScale

    /** The chips, wrapped into rows that fit the width. */
    val allDayRows: List<List<AllDayEvent>> = run {
        val rows = mutableListOf<MutableList<AllDayEvent>>()
        var used = Float.MAX_VALUE
        val available = widthPx - dp(16f)
        data.allDay.forEach { chip ->
            val width = chipWidth(chip, available)
            if (used + width + dp(4f) > available) {
                rows += mutableListOf<AllDayEvent>()
                used = 0f
            }
            rows.last() += chip
            used += width + dp(4f)
        }
        rows
    }

    private fun chipWidth(chip: AllDayEvent, available: Float): Float {
        text.typeface = regular
        text.textSize = sp(12f)
        return min(text.measureText(chip.title) + dp(16f), available)
    }

    fun drawAllDayRow(row: List<AllDayEvent>): Bitmap {
        val height = (ALL_DAY_ROW_DP * density).toInt()
        val bitmap = Bitmap.createBitmap(widthPx, height, Bitmap.Config.RGB_565)
        val canvas = Canvas(bitmap)
        canvas.drawColor(colors.surface)
        val available = widthPx - dp(16f)
        var x = dp(8f)
        row.forEach { chip ->
            val width = chipWidth(chip, available)
            val box = RectF(x, dp(3f), x + width, height - dp(3f))
            fill.color = chip.color
            canvas.drawRoundRect(box, dp(RADIUS_DP), dp(RADIUS_DP), fill)
            val textStart = x + dp(8f)
            drawLine(canvas, chip.title, textStart, box.centerY(), box.right - dp(8f) - textStart, inkOn(chip.color), 12f, regular)
            x += width + dp(4f)
        }
        return bitmap
    }

    fun drawHour(hour: Int): Bitmap {
        val top = (hour * hourHeight)
        val height = hourHeight.toInt()
        val bitmap = Bitmap.createBitmap(widthPx, height, Bitmap.Config.RGB_565)
        val canvas = Canvas(bitmap)
        canvas.drawColor(colors.surface)
        canvas.translate(0f, -top)

        val rowTop = top
        val rowBottom = top + height
        drawGrid(canvas, hour)
        // The next hour's label straddles the seam, so half of it lands here.
        drawGrid(canvas, hour + 1)

        val placed = placement
        placed.visible.forEach { place ->
            if (place.bottom > rowTop && place.top < rowBottom) drawItem(canvas, place)
        }
        placed.overflow.forEach { group ->
            if (group.bottom > rowTop && group.top < rowBottom) drawOverflow(canvas, group)
        }
        // Over the items, since a trip can run back across the one before it.
        placed.trips.forEach { trip ->
            if (trip.box.bottom > rowTop && trip.box.top < rowBottom) drawTravel(canvas, trip)
        }
        data.nowMinutes()?.let { minutes ->
            val y = minutes * hourHeight / 60f
            if (y + dp(5f) > rowTop && y - dp(5f) < rowBottom) drawNow(canvas, y)
        }
        return bitmap
    }

    /** The hour worth showing right now: the current one, or 8am on another day. */
    fun scrollHour(): Int =
        max(0, (data.nowMinutes() ?: (8 * 60 - data.dayStartMinutes)) / 60)

    private fun drawGrid(canvas: Canvas, hour: Int) {
        val y = hour * hourHeight
        stroke.color = colors.line
        stroke.strokeWidth = max(1f, dp(0.75f))
        canvas.drawLine(laneStart - dp(2f), y, widthPx.toFloat(), y, stroke)
        val label = data.hours.getOrNull(hour) ?: return
        text.typeface = regular
        text.textSize = sp(11f)
        text.color = colors.muted
        text.textAlign = Paint.Align.RIGHT
        // The first label would be cut by the top of the list.
        val baseline = if (hour == 0) y + text.textSize + dp(1f) else y + text.textSize * 0.36f
        canvas.drawText(label, gutter - dp(2f), baseline, text)
        text.textAlign = Paint.Align.LEFT
    }

    private fun drawNow(canvas: Canvas, y: Float) {
        stroke.color = colors.now
        stroke.strokeWidth = dp(1.5f)
        canvas.drawLine(laneStart, y, widthPx.toFloat(), y, stroke)
        fill.color = colors.now
        canvas.drawCircle(laneStart, y, dp(4f), fill)
    }

    private class Place(val item: WidgetItem, val box: RectF) {
        val top get() = box.top
        val bottom get() = box.bottom
    }

    private class Overflow(val box: RectF, val count: Int) {
        val top get() = box.top
        val bottom get() = box.bottom
    }

    private class Trip(val travel: WidgetTravel, val box: RectF)

    private class Placed(
        val visible: List<Place>,
        val overflow: List<Overflow>,
        val trips: List<Trip>,
    )

    private val placement: Placed by lazy {
        val visible = mutableListOf<Place>()
        val hidden = mutableListOf<WidgetItem>()
        data.items.forEach { item ->
            val columns = min(item.columns, maxColumns)
            // With too many at once, the last column that fits holds the count.
            if (item.columns > maxColumns && item.column >= maxColumns - 1) {
                hidden += item
            } else {
                visible += Place(item, boxFor(item.start, item.end, item.column, columns))
            }
        }

        val overflow = mutableListOf<Overflow>()
        var start = 0
        var end = 0
        var count = 0
        hidden.sortedBy { it.start }.forEach { item ->
            if (count > 0 && item.start >= end) {
                overflow += Overflow(boxFor(start, end, maxColumns - 1, maxColumns), count)
                count = 0
            }
            if (count == 0) {
                start = item.start
                end = item.end
            }
            end = max(end, item.end)
            count++
        }
        if (count > 0) overflow += Overflow(boxFor(start, end, maxColumns - 1, maxColumns), count)
        // A trip to an item folded into "+N" has nothing on screen to hang off.
        val trips = visible.mapNotNull { place -> place.item.travel?.let { tripFor(it, place.box) } }
        Placed(visible, overflow, trips)
    }

    /** Ends where the item starts and lasts as long as the trip takes. */
    private fun tripFor(travel: WidgetTravel, item: RectF): Trip {
        val height = max(travel.minutes * hourHeight / 60f, dp(TRAVEL_MIN_DP))
        val top = max(0f, item.top - dp(ITEM_GAP_DP) - height)
        val width = if (travel.halfWidth) item.width() / 2 else item.width()
        return Trip(travel, RectF(item.left, top, item.left + width, top + height))
    }

    private fun boxFor(start: Int, end: Int, column: Int, columns: Int): RectF {
        val width = laneWidth / columns
        val left = laneStart + column * width
        val gap = dp(ITEM_GAP_DP)
        return RectF(
            left + gap / 2,
            start * hourHeight / 60f + gap / 2,
            left + width - gap / 2,
            end * hourHeight / 60f - gap / 2,
        )
    }

    private fun drawItem(canvas: Canvas, place: Place) {
        val item = place.item
        val ink = inkOn(item.color)
        fill.color = item.color
        canvas.drawRoundRect(place.box, dp(RADIUS_DP), dp(RADIUS_DP), fill)

        canvas.save()
        canvas.clipRect(place.box)
        val left = place.box.left + dp(6f)
        val textWidth = place.box.right - dp(4f) - left
        drawLine(canvas, item.title, left, place.box.top + dp(14f), textWidth, ink, 13f, bold)
        if (place.box.height() >= dp(42f) && item.time.isNotEmpty()) {
            drawLine(canvas, item.time, left, place.box.top + dp(31f), textWidth, ink, 11f, regular)
        }
        canvas.restore()
    }

    private fun drawTravel(canvas: Canvas, trip: Trip) {
        fill.color = trip.travel.color
        canvas.drawRoundRect(trip.box, dp(RADIUS_DP), dp(RADIUS_DP), fill)
        stroke.color = 0x73000000
        stroke.strokeWidth = max(1f, dp(0.75f))
        canvas.drawRoundRect(trip.box, dp(RADIUS_DP), dp(RADIUS_DP), stroke)

        canvas.save()
        canvas.clipRect(trip.box)
        val left = trip.box.left + dp(6f)
        drawLine(
            canvas,
            trip.travel.label,
            left,
            trip.box.top + dp(TRAVEL_MIN_DP) / 2,
            trip.box.right - dp(4f) - left,
            inkOn(trip.travel.color),
            11f,
            regular,
        )
        canvas.restore()
    }

    private fun drawOverflow(canvas: Canvas, group: Overflow) {
        fill.color = colors.overflow
        canvas.drawRoundRect(group.box, dp(RADIUS_DP), dp(RADIUS_DP), fill)
        text.typeface = bold
        text.textSize = sp(12f)
        text.color = colors.text
        text.textAlign = Paint.Align.CENTER
        canvas.drawText("+${group.count}", group.box.centerX(), group.box.top + dp(18f), text)
        text.textAlign = Paint.Align.LEFT
    }

    /** One ellipsized line, vertically centred on [centre]. */
    private fun drawLine(
        canvas: Canvas,
        value: String,
        left: Float,
        centre: Float,
        width: Float,
        color: Int,
        size: Float,
        face: Typeface,
    ) {
        if (width <= 0f) return
        text.typeface = face
        text.textSize = sp(size)
        text.color = color
        val line = TextUtils.ellipsize(value.replace('\n', ' '), text, width, TextUtils.TruncateAt.END)
        val metrics = text.fontMetrics
        val baseline = centre - (metrics.ascent + metrics.descent) / 2
        canvas.drawText(line, 0, line.length, left, baseline, text)
    }

    private fun inkOn(color: Int): Int {
        val channel = { value: Int ->
            val c = value / 255.0
            if (c <= 0.03928) c / 12.92 else Math.pow((c + 0.055) / 1.055, 2.4)
        }
        val luminance = 0.2126 * channel(Color.red(color)) +
            0.7152 * channel(Color.green(color)) +
            0.0722 * channel(Color.blue(color))
        return if (luminance > 0.4) 0xFF1B1D23.toInt() else Color.WHITE
    }
}
