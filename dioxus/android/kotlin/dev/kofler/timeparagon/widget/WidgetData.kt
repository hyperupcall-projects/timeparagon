package dev.kofler.timeparagon.widget

import android.content.Context
import android.graphics.Color
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.text.SimpleDateFormat
import java.util.Calendar
import java.util.Date
import java.util.Locale

/** The day as `dioxus/src/widget.rs` writes it. */
class WidgetDay(
    /** `yyyy-MM-dd`. */
    val day: String,
    val dayStartMinutes: Int,
    val updated: String,
    val hours: List<String>,
    val allDay: List<AllDayEvent>,
    val items: List<WidgetItem>,
) {
    /** Minutes into this day right now, or null when it is not today. */
    fun nowMinutes(): Int? {
        val shifted = Calendar.getInstance()
        shifted.add(Calendar.MINUTE, -dayStartMinutes)
        if (DAY_FORMAT.format(shifted.time) != day) return null
        return shifted.get(Calendar.HOUR_OF_DAY) * 60 + shifted.get(Calendar.MINUTE)
    }

    /** For example "Sun, Nov 1". */
    fun heading(): String = heading(DAY_FORMAT.parse(day) ?: Date())

    companion object {
        private val DAY_FORMAT = SimpleDateFormat("yyyy-MM-dd", Locale.US)

        fun heading(date: Date): String =
            SimpleDateFormat("EEE, MMM d", Locale.getDefault()).format(date)

        fun load(context: Context): WidgetDay? = try {
            parse(JSONObject(File(context.filesDir, "widget-day.json").readText()))
        } catch (_: Exception) {
            // Not written yet, or from a version that wrote something else.
            null
        }

        private fun parse(json: JSONObject) = WidgetDay(
            day = json.getString("day"),
            dayStartMinutes = json.getInt("day_start_minutes"),
            updated = json.getString("updated"),
            hours = json.getJSONArray("hours").map { array, i -> array.getString(i) },
            allDay = json.getJSONArray("all_day").map { array, i ->
                val event = array.getJSONObject(i)
                AllDayEvent(event.getString("title"), Color.parseColor(event.getString("color")))
            },
            items = json.getJSONArray("items").map { array, i ->
                val item = array.getJSONObject(i)
                WidgetItem(
                    title = item.getString("title"),
                    time = item.getString("time"),
                    color = Color.parseColor(item.getString("color")),
                    start = item.getInt("start"),
                    end = item.getInt("end"),
                    column = item.getInt("column"),
                    columns = item.getInt("columns"),
                    travel = item.optJSONObject("travel")?.let { travel ->
                        WidgetTravel(
                            label = travel.getString("label"),
                            color = Color.parseColor(travel.getString("color")),
                            minutes = travel.getInt("minutes"),
                            halfWidth = travel.getBoolean("half_width"),
                        )
                    },
                )
            },
        )

        private fun <T> JSONArray.map(read: (JSONArray, Int) -> T): List<T> =
            (0 until length()).map { read(this, it) }
    }
}

class AllDayEvent(val title: String, val color: Int)

class WidgetItem(
    val title: String,
    val time: String,
    val color: Int,
    val start: Int,
    val end: Int,
    val column: Int,
    val columns: Int,
    /** The trip that leads to this item. */
    val travel: WidgetTravel?,
)

class WidgetTravel(
    val label: String,
    val color: Int,
    /** How long the trip takes, or 0 when the desktop has not worked it out. */
    val minutes: Int,
    val halfWidth: Boolean,
)
