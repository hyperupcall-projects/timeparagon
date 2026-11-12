package dev.kofler.timeparagon.reminders

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Build
import android.util.Log
import dev.kofler.timeparagon.R

/**
 * The notification kept up while sleep is being timed, as `dioxus/src/sleep.rs`
 * asks for it. While sleeping it is ongoing, with a running clock from when
 * sleep began; finishing replaces it with how long was slept.
 */
object SleepNotification {
    const val ACTION = "dev.kofler.timeparagon.reminders.SLEEP"

    /** `sleeping`, `slept` or `none`, which takes the notification down. */
    const val EXTRA_STATE = "state"
    const val EXTRA_TITLE = "title"
    const val EXTRA_BODY = "body"

    /** When sleep began, in milliseconds since the epoch. */
    const val EXTRA_SINCE = "since"

    private const val TAG = "TimeParagonSleep"
    private const val CHANNEL = "sleep_timer"

    /** One notification, replaced in place as the night moves on. */
    private const val ID = 0x51ee9

    fun handle(context: Context, intent: Intent) {
        val manager = context.getSystemService(NotificationManager::class.java)
        val state = intent.getStringExtra(EXTRA_STATE)
        if (state != "sleeping" && state != "slept") {
            // Only the running timer goes; a summary of the night stays
            // until it is dismissed.
            val ongoing = Build.VERSION.SDK_INT < Build.VERSION_CODES.M ||
                manager.activeNotifications.any {
                    it.id == ID && it.notification.flags and Notification.FLAG_ONGOING_EVENT != 0
                }
            if (ongoing) manager.cancel(ID)
            return
        }
        if (!manager.areNotificationsEnabled()) {
            Log.w(TAG, "Notifications are off; not posting the sleep timer")
            return
        }
        ensureChannel(manager)
        val sleeping = state == "sleeping"
        val title = intent.getStringExtra(EXTRA_TITLE).orEmpty()
        val body = intent.getStringExtra(EXTRA_BODY).orEmpty()
        val since = intent.getStringExtra(EXTRA_SINCE)?.toLongOrNull()

        val builder = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            Notification.Builder(context, CHANNEL)
        } else {
            @Suppress("DEPRECATION")
            Notification.Builder(context).setPriority(Notification.PRIORITY_LOW)
        }
        builder
            .setSmallIcon(R.drawable.timeparagon_reminder)
            .setContentTitle(title)
            .setContentText(body)
            .setStyle(Notification.BigTextStyle().bigText(body))
            .setOnlyAlertOnce(true)
            .setOngoing(sleeping)
            .setAutoCancel(!sleeping)
            .setShowWhen(true)
        if (sleeping && since != null) {
            // Counts up from bedtime on its own, so the app need not refresh it.
            builder.setWhen(since).setUsesChronometer(true)
        } else {
            builder.setWhen(System.currentTimeMillis())
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.LOLLIPOP) {
            builder.setCategory(
                if (sleeping) Notification.CATEGORY_STATUS else Notification.CATEGORY_EVENT,
            )
        }
        context.packageManager.getLaunchIntentForPackage(context.packageName)?.let {
            builder.setContentIntent(
                PendingIntent.getActivity(
                    context, ID, it,
                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
                ),
            )
        }
        manager.notify(ID, builder.build())
    }

    /** Quiet: the timer is something to glance at, never something to hear. */
    private fun ensureChannel(manager: NotificationManager) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        if (manager.getNotificationChannel(CHANNEL) != null) return
        val channel = NotificationChannel(
            CHANNEL, "Sleep timer", NotificationManager.IMPORTANCE_LOW,
        )
        channel.description = "Shown while sleep is being timed, then how long you slept"
        channel.setSound(null, null)
        channel.enableVibration(false)
        manager.createNotificationChannel(channel)
    }
}
