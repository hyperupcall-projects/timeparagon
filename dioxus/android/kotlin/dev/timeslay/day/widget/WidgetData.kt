package dev.timeslay.day.widget

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
    val looseTasks: List<WidgetTask>,
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
            looseTasks = json.getJSONArray("loose_tasks").map { array, i ->
                parseTask(array.getJSONObject(i))
            },
            items = json.getJSONArray("items").map { array, i ->
                val item = array.getJSONObject(i)
                WidgetItem(
                    kind = item.getString("kind"),
                    title = item.getString("title"),
                    time = item.getString("time"),
                    color = Color.parseColor(item.getString("color")),
                    start = item.getInt("start"),
                    end = item.getInt("end"),
                    column = item.getInt("column"),
                    columns = item.getInt("columns"),
                    done = item.getBoolean("done"),
                    overdue = item.getBoolean("overdue"),
                    tasks = item.getJSONArray("tasks").map { tasks, t ->
                        parseTask(tasks.getJSONObject(t))
                    },
                )
            },
        )

        private fun parseTask(task: JSONObject) = WidgetTask(
            task.getString("title"),
            task.getBoolean("done"),
            task.getBoolean("overdue"),
        )

        private fun <T> JSONArray.map(read: (JSONArray, Int) -> T): List<T> =
            (0 until length()).map { read(this, it) }
    }
}

class AllDayEvent(val title: String, val color: Int)

class WidgetTask(val title: String, val done: Boolean, val overdue: Boolean)

class WidgetItem(
    val kind: String,
    val title: String,
    val time: String,
    val color: Int,
    val start: Int,
    val end: Int,
    val column: Int,
    val columns: Int,
    val done: Boolean,
    val overdue: Boolean,
    val tasks: List<WidgetTask>,
)
