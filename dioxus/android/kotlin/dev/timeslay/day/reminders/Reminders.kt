package dev.timeslay.day.reminders

import android.app.AlarmManager
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationChannelGroup
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.graphics.Color
import android.media.AudioAttributes
import android.media.RingtoneManager
import android.net.Uri
import android.os.Build
import android.provider.Settings
import android.util.Log
import dev.timeslay.day.R
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/** A reminder's sound, as `dioxus/src/reminders.rs` describes it. */
class SoundSpec(
    /** Changes whenever the sound does, and is part of the channel id. */
    val key: String,
    /** `default`, `silent`, `tone` or `device`. */
    val kind: String,
    val resource: String,
    val uri: String,
)

class CategorySpec(
    val id: String,
    val label: String,
    val enabled: Boolean,
    val color: Int,
    val sound: SoundSpec,
)

class ReminderItem(
    val key: String,
    val category: String,
    val title: String,
    val body: String,
    /** Milliseconds since the epoch. */
    val at: Long,
)

class Plan(
    val enabled: Boolean,
    val categories: Map<String, CategorySpec>,
    val reminders: List<ReminderItem>,
)

/**
 * Schedules the plan the app writes to `reminders.json` with AlarmManager,
 * and posts each reminder on its category's notification channel.
 *
 * Android fixes a channel's sound when the channel is created, so every
 * (category, sound) pair gets its own channel and the old one is deleted when
 * the sound changes.
 */
object Reminders {
    const val ACTION_SYNC = "dev.timeslay.day.reminders.SYNC"
    const val ACTION_FIRE = "dev.timeslay.day.reminders.FIRE"
    const val ACTION_TEST = "dev.timeslay.day.reminders.TEST"
    const val ACTION_DEVICE_INFO = "dev.timeslay.day.reminders.DEVICE_INFO"

    const val EXTRA_KEY = "key"
    const val EXTRA_CATEGORY = "category"
    const val EXTRA_TITLE = "title"
    const val EXTRA_BODY = "body"
    const val EXTRA_AT = "at"

    private const val TAG = "TimeSlayReminders"
    private const val PREFS = "timeslay-reminders"
    private const val PREF_SCHEDULED = "scheduled"
    private const val PREF_POSTED = "posted"
    private const val CHANNEL_PREFIX = "reminder_"
    private const val GROUP = "reminders"
    private const val SCHEME = "timeslay-reminder"

    /** Matches `LATE_GRACE_MINUTES` in core: older reminders are dropped. */
    private const val LATE_GRACE_MILLIS = 5 * 60 * 1000L

    fun load(context: Context): Plan? = try {
        parse(JSONObject(File(context.filesDir, "reminders.json").readText()))
    } catch (_: Exception) {
        // Not written yet, or from a version that wrote something else.
        null
    }

    private fun parse(json: JSONObject): Plan {
        val categories = json.getJSONArray("categories").map { array, i ->
            val category = array.getJSONObject(i)
            val sound = category.getJSONObject("sound")
            CategorySpec(
                id = category.getString("id"),
                label = category.getString("label"),
                enabled = category.getBoolean("enabled"),
                color = Color.parseColor(category.getString("color")),
                sound = SoundSpec(
                    key = sound.getString("key"),
                    kind = sound.getString("kind"),
                    resource = sound.optString("resource"),
                    uri = sound.optString("uri"),
                ),
            )
        }
        return Plan(
            enabled = json.getBoolean("enabled"),
            categories = categories.associateBy { it.id },
            reminders = json.getJSONArray("reminders").map { array, i ->
                val item = array.getJSONObject(i)
                ReminderItem(
                    key = item.getString("key"),
                    category = item.getString("category"),
                    title = item.getString("title"),
                    body = item.getString("body"),
                    at = item.getLong("at"),
                )
            },
        )
    }

    private fun <T> JSONArray.map(read: (JSONArray, Int) -> T): List<T> =
        (0 until length()).map { read(this, it) }

    /** Replaces every scheduled reminder with the ones in the current plan. */
    fun sync(context: Context) {
        val plan = load(context)
        val alarms = context.getSystemService(AlarmManager::class.java)
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

        for (key in prefs.getStringSet(PREF_SCHEDULED, emptySet()).orEmpty()) {
            PendingIntent.getBroadcast(
                context, 0, fireIntent(context, key),
                PendingIntent.FLAG_NO_CREATE or PendingIntent.FLAG_IMMUTABLE,
            )?.let {
                alarms.cancel(it)
                it.cancel()
            }
        }

        if (plan == null || !plan.enabled) {
            prefs.edit().remove(PREF_SCHEDULED).apply()
            return
        }
        ensureChannels(context, plan)
        // The widget's job keeps the calendar, and so this plan, fresh while
        // the app is closed.
        runCatching { dev.timeslay.day.widget.WidgetRefreshService.schedule(context) }

        val now = System.currentTimeMillis()
        val posted = prefs.getStringSet(PREF_POSTED, emptySet()).orEmpty()
        val scheduled = mutableSetOf<String>()
        for (item in plan.reminders) {
            if (plan.categories[item.category]?.enabled != true) continue
            if (item.key in posted || item.at < now - LATE_GRACE_MILLIS) continue
            val intent = fireIntent(context, item.key)
                .putExtra(EXTRA_KEY, item.key)
                .putExtra(EXTRA_CATEGORY, item.category)
                .putExtra(EXTRA_TITLE, item.title)
                .putExtra(EXTRA_BODY, item.body)
                .putExtra(EXTRA_AT, item.at)
            val pending = PendingIntent.getBroadcast(
                context, 0, intent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
            schedule(alarms, item.at, pending)
            scheduled += item.key
        }

        // Posted keys only need remembering while they are still in the plan.
        val planned = plan.reminders.mapTo(HashSet()) { it.key }
        prefs.edit()
            .putStringSet(PREF_SCHEDULED, scheduled)
            .putStringSet(PREF_POSTED, posted.filterTo(HashSet()) { it in planned })
            .apply()
        Log.i(TAG, "Scheduled ${scheduled.size} reminders")
    }

    private fun schedule(alarms: AlarmManager, at: Long, pending: PendingIntent) {
        val exact = Build.VERSION.SDK_INT < Build.VERSION_CODES.S || alarms.canScheduleExactAlarms()
        if (exact) {
            alarms.setExactAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, pending)
        } else {
            // Without the exact alarm permission Android may defer it by a few
            // minutes, which is still better than no reminder.
            alarms.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, pending)
        }
    }

    /** One intent per reminder: the data URI keeps their PendingIntents apart. */
    private fun fireIntent(context: Context, key: String): Intent =
        Intent(ACTION_FIRE)
            .setClass(context, ReminderReceiver::class.java)
            .setData(Uri.fromParts(SCHEME, key, null))

    /** Posts a reminder that came due, unless it was switched off since. */
    fun fire(context: Context, intent: Intent) {
        val key = intent.getStringExtra(EXTRA_KEY) ?: return
        val plan = load(context) ?: return
        val category = plan.categories[intent.getStringExtra(EXTRA_CATEGORY)] ?: return
        if (!plan.enabled || !category.enabled) return

        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val posted = prefs.getStringSet(PREF_POSTED, emptySet()).orEmpty()
        if (key in posted) return
        prefs.edit().putStringSet(PREF_POSTED, posted + key).apply()

        ensureChannels(context, plan)
        post(
            context,
            category,
            intent.getStringExtra(EXTRA_TITLE).orEmpty(),
            intent.getStringExtra(EXTRA_BODY).orEmpty(),
            intent.getLongExtra(EXTRA_AT, System.currentTimeMillis()),
            key.hashCode(),
        )
    }

    /** Posts a sample of a category's reminder, from the settings sheet. */
    fun test(context: Context, categoryId: String?) {
        val plan = load(context) ?: return
        val category = plan.categories[categoryId] ?: return
        ensureChannels(context, plan)
        post(
            context,
            category,
            "${category.label} reminder",
            "This is how TimeSlay reminds you.",
            System.currentTimeMillis(),
            "test-${category.id}".hashCode(),
        )
    }

    private fun channelId(category: CategorySpec) =
        "$CHANNEL_PREFIX${category.id}_${category.sound.key}"

    private fun soundUri(context: Context, sound: SoundSpec): Uri? = when (sound.kind) {
        "silent" -> null
        "tone" -> Uri.parse("android.resource://${context.packageName}/raw/${sound.resource}")
        "device" -> Uri.parse(sound.uri)
        else -> Settings.System.DEFAULT_NOTIFICATION_URI
    }

    private val audioAttributes: AudioAttributes =
        AudioAttributes.Builder()
            .setUsage(AudioAttributes.USAGE_NOTIFICATION_EVENT)
            .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
            .build()

    /** Creates a channel per category for its current sound, and removes stale ones. */
    private fun ensureChannels(context: Context, plan: Plan) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannelGroup(NotificationChannelGroup(GROUP, "Reminders"))

        val wanted = plan.categories.values.mapTo(HashSet()) { channelId(it) }
        for (channel in manager.notificationChannels) {
            if (channel.id.startsWith(CHANNEL_PREFIX) && channel.id !in wanted) {
                manager.deleteNotificationChannel(channel.id)
            }
        }
        for (category in plan.categories.values) {
            val id = channelId(category)
            if (manager.getNotificationChannel(id) != null) continue
            val channel = NotificationChannel(
                id, "${category.label} reminders", NotificationManager.IMPORTANCE_HIGH,
            )
            channel.group = GROUP
            channel.description = "Reminders for ${category.label.lowercase()} on your calendar"
            channel.enableLights(true)
            channel.lightColor = category.color
            channel.enableVibration(category.sound.kind != "silent")
            channel.setSound(soundUri(context, category.sound), audioAttributes)
            manager.createNotificationChannel(channel)
        }
    }

    private fun post(
        context: Context,
        category: CategorySpec,
        title: String,
        body: String,
        at: Long,
        id: Int,
    ) {
        val manager = context.getSystemService(NotificationManager::class.java)
        if (!manager.areNotificationsEnabled()) {
            Log.w(TAG, "Notifications are off; not posting a reminder")
            return
        }
        val launch = context.packageManager.getLaunchIntentForPackage(context.packageName)
        val content = launch?.let {
            PendingIntent.getActivity(
                context, 0, it,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
        }
        val builder = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            Notification.Builder(context, channelId(category))
        } else {
            @Suppress("DEPRECATION")
            Notification.Builder(context).apply {
                setPriority(Notification.PRIORITY_HIGH)
                when (category.sound.kind) {
                    "silent" -> {}
                    "default" -> setDefaults(Notification.DEFAULT_SOUND or Notification.DEFAULT_VIBRATE)
                    else -> {
                        setSound(soundUri(context, category.sound), audioAttributes)
                        setDefaults(Notification.DEFAULT_VIBRATE)
                    }
                }
            }
        }
        builder
            .setSmallIcon(R.drawable.timeslay_reminder)
            .setContentTitle(title)
            .setContentText(body)
            .setStyle(Notification.BigTextStyle().bigText(body))
            .setCategory(Notification.CATEGORY_REMINDER)
            .setColor(category.color)
            .setWhen(at)
            .setShowWhen(true)
            .setAutoCancel(true)
        content?.let { builder.setContentIntent(it) }
        manager.notify(id, builder.build())
    }

    /**
     * Writes `reminder-device.json`: whether notifications are allowed, and the
     * device's own notification sounds for the sound picker.
     */
    fun writeDeviceInfo(context: Context) {
        val manager = context.getSystemService(NotificationManager::class.java)
        val sounds = JSONArray()
        runCatching {
            val ringtones = RingtoneManager(context)
            ringtones.setType(RingtoneManager.TYPE_NOTIFICATION)
            val cursor = ringtones.cursor
            val seen = HashSet<String>()
            while (cursor.moveToNext()) {
                val uri = ringtones.getRingtoneUri(cursor.position)?.toString() ?: continue
                if (!seen.add(uri)) continue
                sounds.put(
                    JSONObject()
                        .put("name", cursor.getString(RingtoneManager.TITLE_COLUMN_INDEX))
                        .put("uri", uri),
                )
            }
        }.onFailure { Log.w(TAG, "Could not list the notification sounds", it) }
        val json = JSONObject()
            .put("notifications_enabled", manager.areNotificationsEnabled())
            .put("sounds", sounds)
        val target = File(context.filesDir, "reminder-device.json")
        val temporary = File(context.filesDir, "reminder-device.json.tmp")
        temporary.writeText(json.toString())
        temporary.renameTo(target)
    }
}
