package dev.timeslay.day.reminders

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log

/**
 * Everything reminders react to: a new plan from the app, an alarm coming
 * due, a test from the settings sheet, the sleep timer starting or
 * finishing, and the system events that clear alarms (a reboot, an app
 * update, the clock changing).
 */
class ReminderReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val pending = goAsync()
        // Reading files and the ringtone list stays off the main thread.
        Thread {
            try {
                when (intent.action) {
                    Reminders.ACTION_FIRE -> Reminders.fire(context, intent)
                    Reminders.ACTION_TEST -> Reminders.test(
                        context, intent.getStringExtra(Reminders.EXTRA_CATEGORY),
                    )
                    Reminders.ACTION_DEVICE_INFO -> Reminders.writeDeviceInfo(context)
                    SleepNotification.ACTION -> SleepNotification.handle(context, intent)
                    Reminders.ACTION_SYNC,
                    Intent.ACTION_BOOT_COMPLETED,
                    Intent.ACTION_MY_PACKAGE_REPLACED,
                    Intent.ACTION_TIME_CHANGED,
                    Intent.ACTION_TIMEZONE_CHANGED -> Reminders.sync(context)
                }
            } catch (error: Exception) {
                Log.e("TimeSlayReminders", "Could not handle ${intent.action}", error)
            } finally {
                pending.finish()
            }
        }.start()
    }
}
