package dev.timeslay.day.widget

import android.app.job.JobInfo
import android.app.job.JobParameters
import android.app.job.JobScheduler
import android.app.job.JobService
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.util.Log

/** Syncs today's calendar for the widget while the app is closed. */
class WidgetRefreshService : JobService() {
    override fun onStartJob(params: JobParameters): Boolean {
        Thread {
            val done = runCatching {
                System.loadLibrary("main")
                nativeRefresh(applicationContext)
            }.onFailure { Log.e(TAG, "Could not refresh the widget", it) }.getOrDefault(false)
            Log.i(TAG, if (done) "Refreshed" else "Refresh failed; retrying later")
            if (done) {
                sendBroadcast(
                    Intent(DayWidgetProvider.ACTION_DATA_CHANGED).setPackage(packageName),
                )
            }
            jobFinished(params, !done)
        }.start()
        return true
    }

    /** The sync finishes its own writes; a stopped job simply runs again later. */
    override fun onStopJob(params: JobParameters): Boolean = true

    private external fun nativeRefresh(context: Context): Boolean

    companion object {
        const val JOB_ID = 7301
        private const val TAG = "TimeSlayWidget"
        private const val PERIOD_MILLIS = 30 * 60 * 1000L

        fun schedule(context: Context) {
            val scheduler = context.getSystemService(JobScheduler::class.java)
            if (scheduler.getPendingJob(JOB_ID) != null) return
            scheduler.schedule(
                JobInfo.Builder(JOB_ID, ComponentName(context, WidgetRefreshService::class.java))
                    .setRequiredNetworkType(JobInfo.NETWORK_TYPE_ANY)
                    .setPeriodic(PERIOD_MILLIS)
                    .build(),
            )
        }
    }
}
