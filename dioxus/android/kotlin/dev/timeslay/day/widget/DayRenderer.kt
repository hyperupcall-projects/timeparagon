package dev.timeslay.day.widget

import android.content.Context
import android.content.res.Configuration
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Path
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

/** Below this a column cannot show a useful word, so the rest collapse into "+N". */
private const val MIN_COLUMN_DP = 58f

/** A block narrower than this stacks its tasks under the title. */
private const val SPLIT_BLOCK_DP = 150f

private const val TASK_LINE_DP = 18f

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
            val dark = when (preference) {
                "light" -> false
                "dark" -> true
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

/** A row of chips above the timeline. */
class AllDayChip(val title: String, val color: Int?, val overdue: Boolean)

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
    val allDayRows: List<List<AllDayChip>> = run {
        val chips = data.allDay.map { AllDayChip(it.title, it.color, false) } +
            data.looseTasks.map { AllDayChip(it.title, null, it.overdue) }
        val rows = mutableListOf<MutableList<AllDayChip>>()
        var used = Float.MAX_VALUE
        val available = widthPx - dp(16f)
        chips.forEach { chip ->
            val width = chipWidth(chip, available)
            if (used + width + dp(4f) > available) {
                rows += mutableListOf<AllDayChip>()
                used = 0f
            }
            rows.last() += chip
            used += width + dp(4f)
        }
        rows
    }

    private fun chipWidth(chip: AllDayChip, available: Float): Float {
        text.typeface = regular
        text.textSize = sp(12f)
        val circle = if (chip.color == null) dp(15f) else 0f
        return min(text.measureText(chip.title) + dp(16f) + circle, available)
    }

    fun drawAllDayRow(row: List<AllDayChip>): Bitmap {
        val height = (ALL_DAY_ROW_DP * density).toInt()
        val bitmap = Bitmap.createBitmap(widthPx, height, Bitmap.Config.RGB_565)
        val canvas = Canvas(bitmap)
        canvas.drawColor(colors.surface)
        val available = widthPx - dp(16f)
        var x = dp(8f)
        row.forEach { chip ->
            val width = chipWidth(chip, available)
            val box = RectF(x, dp(3f), x + width, height - dp(3f))
            val background = chip.color ?: if (chip.overdue) tint(colors.now, 0.16f) else colors.overflow
            val ink = when {
                chip.color != null -> inkOn(chip.color)
                chip.overdue -> colors.now
                else -> colors.text
            }
            fill.color = background
            canvas.drawRoundRect(box, dp(RADIUS_DP), dp(RADIUS_DP), fill)
            var textStart = x + dp(8f)
            if (chip.color == null) {
                drawCircle(canvas, textStart + dp(5f), box.centerY(), false, ink)
                textStart += dp(15f)
            }
            drawLine(canvas, chip.title, textStart, box.centerY(), box.right - dp(8f) - textStart, ink, 12f, regular)
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

    private class Placed(val visible: List<Place>, val overflow: List<Overflow>)

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
        Placed(visible, overflow)
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
        if (item.kind == "block") {
            drawBlock(canvas, place.box, item)
            return
        }
        val color = if (item.done) tint(item.color, 0.35f) else item.color
        val ink = if (item.done) colors.muted else inkOn(color)
        fill.color = color
        canvas.drawRoundRect(place.box, dp(RADIUS_DP), dp(RADIUS_DP), fill)

        canvas.save()
        canvas.clipRect(place.box)
        var left = place.box.left + dp(6f)
        val right = place.box.right - dp(4f)
        if (item.kind == "task") {
            val centre = place.box.top + dp(14f)
            drawCircle(canvas, left + dp(5f), centre, item.done, if (item.overdue) colors.now else ink)
            left += dp(14f)
        }
        val textWidth = right - left
        val tall = place.box.height() >= dp(42f)
        drawLine(canvas, item.title, left, place.box.top + dp(14f), textWidth, ink, 13f, bold, item.done)
        if (tall && item.time.isNotEmpty()) {
            drawLine(canvas, item.time, left, place.box.top + dp(31f), textWidth, ink, 11f, regular)
        }
        canvas.restore()
    }

    /**
     * A block is a solid panel naming it and a tinted panel listing its tasks,
     * side by side when the block is wide and stacked when it is not.
     */
    private fun drawBlock(canvas: Canvas, box: RectF, item: WidgetItem) {
        val radius = dp(RADIUS_DP)
        val ink = inkOn(item.color)
        if (item.tasks.isEmpty()) {
            fill.color = item.color
            canvas.drawRoundRect(box, radius, radius, fill)
            drawBlockTitle(canvas, box, item, ink)
            return
        }

        fill.color = tint(item.color, 0.22f)
        canvas.drawRoundRect(box, radius, radius, fill)
        val split = box.width() >= dp(SPLIT_BLOCK_DP)
        // Clipped so the solid panel keeps the block's rounded corners.
        val outline = Path().apply { addRoundRect(box, radius, radius, Path.Direction.CW) }
        canvas.save()
        canvas.clipPath(outline)
        val titlePanel = if (split) {
            RectF(box.left, box.top, box.left + box.width() * 0.42f, box.bottom)
        } else {
            RectF(box.left, box.top, box.right, min(box.bottom, box.top + dp(if (box.height() >= dp(70f)) 44f else 26f)))
        }
        fill.color = item.color
        canvas.drawRect(titlePanel, fill)
        drawBlockTitle(canvas, titlePanel, item, ink)
        val tasks = if (split) {
            RectF(titlePanel.right, box.top, box.right, box.bottom)
        } else {
            RectF(box.left, titlePanel.bottom, box.right, box.bottom)
        }
        drawBlockTasks(canvas, tasks, item, ink)
        canvas.restore()
    }

    private fun drawBlockTitle(canvas: Canvas, panel: RectF, item: WidgetItem, ink: Int) {
        val left = panel.left + dp(6f)
        val width = panel.width() - dp(10f)
        drawLine(canvas, item.title, left, panel.top + dp(14f), width, ink, 13f, bold)
        if (panel.height() >= dp(38f)) {
            drawLine(canvas, item.time, left, panel.top + dp(31f), width, ink, 11f, regular)
        }
    }

    private fun drawBlockTasks(canvas: Canvas, panel: RectF, item: WidgetItem, blockInk: Int) {
        if (panel.height() < dp(TASK_LINE_DP)) return
        val room = floor((panel.height() - dp(4f)) / dp(TASK_LINE_DP)).toInt()
        val needsCount = item.tasks.size > room
        val shown = if (needsCount) max(0, room - 1) else item.tasks.size
        val left = panel.left + dp(6f)
        val width = panel.width() - dp(21f)
        var centre = panel.top + dp(2f) + dp(TASK_LINE_DP) / 2
        item.tasks.take(shown).forEach { task ->
            val ink = when {
                task.done -> colors.muted
                task.overdue -> colors.now
                else -> colors.text
            }
            drawCircle(canvas, left + dp(5f), centre, task.done, ink)
            drawLine(canvas, task.title, left + dp(15f), centre, width, ink, 12f, regular, task.done)
            centre += dp(TASK_LINE_DP)
        }
        if (needsCount) {
            val rest = item.tasks.size - shown
            val label = if (shown == 0) taskCount(rest) else "+$rest more"
            drawLine(canvas, label, left, centre, panel.width() - dp(10f), colors.muted, 11f, bold)
        }
    }

    private fun taskCount(count: Int) = if (count == 1) "1 task" else "$count tasks"

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

    private fun drawCircle(canvas: Canvas, x: Float, y: Float, done: Boolean, color: Int) {
        val radius = dp(4.5f)
        if (done) {
            fill.color = color
            canvas.drawCircle(x, y, radius, fill)
        } else {
            stroke.color = color
            stroke.strokeWidth = dp(1.2f)
            canvas.drawCircle(x, y, radius, stroke)
        }
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
        strike: Boolean = false,
    ) {
        if (width <= 0f) return
        text.typeface = face
        text.textSize = sp(size)
        text.color = color
        text.isStrikeThruText = strike
        val line = TextUtils.ellipsize(value.replace('\n', ' '), text, width, TextUtils.TruncateAt.END)
        val metrics = text.fontMetrics
        val baseline = centre - (metrics.ascent + metrics.descent) / 2
        canvas.drawText(line, 0, line.length, left, baseline, text)
        text.isStrikeThruText = false
    }

    /** [color] mixed into the surface, keeping it readable on either theme. */
    private fun tint(color: Int, amount: Float): Int {
        val mix = { over: Int, under: Int -> (over * amount + under * (1 - amount)).toInt() }
        return Color.rgb(
            mix(Color.red(color), Color.red(colors.surface)),
            mix(Color.green(color), Color.green(colors.surface)),
            mix(Color.blue(color), Color.blue(colors.surface)),
        )
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
