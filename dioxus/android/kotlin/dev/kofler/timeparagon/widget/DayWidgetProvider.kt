package dev.kofler.timeparagon.widget

import android.app.AlarmManager
import android.app.PendingIntent
import android.app.job.JobScheduler
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.res.Configuration
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.widget.RemoteViews
import dev.kofler.timeparagon.R
import java.util.Calendar
import java.util.Date

class DayWidgetProvider : AppWidgetProvider() {
    override fun onReceive(context: Context, intent: Intent) {
        super.onReceive(context, intent)
        when (intent.action) {
            ACTION_DATA_CHANGED -> redraw(context)
            ACTION_TICK -> {
                scheduleTick(context)
                redraw(context)
            }
        }
    }

    override fun onUpdate(context: Context, manager: AppWidgetManager, ids: IntArray) {
        scheduleTick(context)
        WidgetRefreshService.schedule(context)
        redraw(context)
    }

    override fun onAppWidgetOptionsChanged(
        context: Context,
        manager: AppWidgetManager,
        id: Int,
        options: Bundle,
    ) {
        updateAll(context)
        // The rows are drawn for the widget's width, and the launcher is still
        // rebuilding the list at this point, so asking right away is lost.
        val pending = goAsync()
        val handler = Handler(Looper.getMainLooper())
        handler.postDelayed({
            manager.notifyAppWidgetViewDataChanged(id, R.id.day_widget_list)
        }, SCROLL_DELAY_MILLIS)
        // The rebuilt list starts back at the top.
        handler.postDelayed({
            val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            val views = RemoteViews(context.packageName, R.layout.day_widget)
            views.setScrollPosition(R.id.day_widget_list, prefs.getInt("position-$id", 0))
            manager.partiallyUpdateAppWidget(id, views)
            pending.finish()
        }, SCROLL_RETRY_MILLIS)
    }

    override fun onEnabled(context: Context) {
        scheduleTick(context)
        WidgetRefreshService.schedule(context)
    }

    override fun onDisabled(context: Context) {
        context.getSystemService(AlarmManager::class.java).cancel(tickIntent(context))
        context.getSystemService(JobScheduler::class.java).cancel(WidgetRefreshService.JOB_ID)
    }

    private fun redraw(context: Context) {
        updateAll(context)
        redrawRows(context)
        val data = WidgetDay.load(context) ?: return
        val positions = ids(context).associateWith { scrollPosition(context, it, data) }
            .filterValues { it != null }
        if (positions.isEmpty()) return
        // Refreshing the rows cancels a scroll in progress, and a list that has to
        // reconnect to this process ignores one, so it is sent again later.
        val pending = goAsync()
        val handler = Handler(Looper.getMainLooper())
        val scroll = Runnable {
            positions.forEach { (id, position) ->
                val views = RemoteViews(context.packageName, R.layout.day_widget)
                views.setScrollPosition(R.id.day_widget_list, position!!)
                manager(context).partiallyUpdateAppWidget(id, views)
            }
        }
        handler.postDelayed(scroll, SCROLL_DELAY_MILLIS)
        handler.postDelayed({
            scroll.run()
            pending.finish()
        }, SCROLL_RETRY_MILLIS)
    }

    companion object {
        const val ACTION_DATA_CHANGED = "dev.kofler.timeparagon.widget.DATA_CHANGED"
        private const val ACTION_TICK = "dev.kofler.timeparagon.widget.TICK"
        private const val PREFS = "day_widget"
        private const val TICK_MILLIS = 5 * 60 * 1000L
        private const val HEADER_AND_FOOTER_DP = 96
        private const val SCROLLS_PER_HOUR = 2
        private const val SCROLL_DELAY_MILLIS = 1500L
        private const val SCROLL_RETRY_MILLIS = 6000L

        private val manager = { context: Context -> AppWidgetManager.getInstance(context) }

        private fun ids(context: Context): IntArray = manager(context).getAppWidgetIds(
            ComponentName(context, DayWidgetProvider::class.java),
        )

        private fun redrawRows(context: Context) {
            manager(context).notifyAppWidgetViewDataChanged(ids(context), R.id.day_widget_list)
        }

        /** Launchers report the narrowest width for portrait, the widest for landscape. */
        private fun widthDp(context: Context, id: Int): Int {
            val landscape = context.resources.configuration.orientation ==
                Configuration.ORIENTATION_LANDSCAPE
            return manager(context).getAppWidgetOptions(id).getInt(
                if (landscape) {
                    AppWidgetManager.OPTION_APPWIDGET_MAX_WIDTH
                } else {
                    AppWidgetManager.OPTION_APPWIDGET_MIN_WIDTH
                },
            ).takeIf { it > 0 } ?: 300
        }

        fun rendererFor(context: Context, data: WidgetDay, id: Int): DayRenderer {
            val metrics = context.resources.displayMetrics
            return DayRenderer(
                data,
                WidgetColors.of(context),
                (widthDp(context, id) * metrics.density).toInt().coerceAtLeast(1),
                metrics.density,
                context.resources.configuration.fontScale,
            )
        }

        private fun updateAll(context: Context) {
            val data = WidgetDay.load(context)
            val colors = WidgetColors.of(context)
            ids(context).forEach { id ->
                manager(context).updateAppWidget(id, views(context, id, data, colors))
            }
        }

        private fun views(
            context: Context,
            id: Int,
            data: WidgetDay?,
            colors: WidgetColors,
        ): RemoteViews {
            val views = RemoteViews(context.packageName, R.layout.day_widget)
            views.setInt(R.id.day_widget_background, "setColorFilter", colors.surface)
            views.setInt(R.id.day_widget_add_background, "setColorFilter", colors.accent)
            views.setInt(R.id.day_widget_add_icon, "setColorFilter", colors.onAccent)
            views.setTextColor(R.id.day_widget_title, colors.text)
            views.setTextColor(R.id.day_widget_updated, colors.muted)
            views.setTextColor(R.id.day_widget_empty, colors.muted)

            views.setTextViewText(
                R.id.day_widget_title,
                data?.heading() ?: WidgetDay.heading(Date()),
            )
            views.setTextViewText(
                R.id.day_widget_updated,
                data?.let { "Updated ${it.updated}" }.orEmpty(),
            )

            val list = Intent(context, DayWidgetService::class.java)
                .putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, id)
            // Without a distinct URI the system reuses one factory for every widget.
            list.data = Uri.parse(list.toUri(Intent.URI_INTENT_SCHEME))
            views.setRemoteAdapter(R.id.day_widget_list, list)
            views.setEmptyView(R.id.day_widget_list, R.id.day_widget_empty)

            val open = openAppIntent(context, 0, PendingIntent.FLAG_IMMUTABLE)
            views.setOnClickPendingIntent(R.id.day_widget_root, open)
            views.setOnClickPendingIntent(R.id.day_widget_header, open)
            views.setOnClickPendingIntent(R.id.day_widget_footer, open)
            views.setOnClickPendingIntent(R.id.day_widget_empty, open)
            views.setOnClickPendingIntent(R.id.day_widget_add, open)
            // A template must stay mutable, because each row's fill-in intent
            // is merged into it.
            views.setPendingIntentTemplate(
                R.id.day_widget_list,
                openAppIntent(
                    context,
                    1,
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                        PendingIntent.FLAG_MUTABLE
                    } else {
                        0
                    },
                ),
            )

            return views
        }

        /**
         * Follows the clock, but only at the start of each hour: anything more
         * often would pull the list away from wherever the user has scrolled
         * it. A list that had to reconnect to this process drops the scroll, so
         * it gets a second chance on the following redraw.
         */
        private fun scrollPosition(
            context: Context,
            id: Int,
            data: WidgetDay,
        ): Int? {
            val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            val stamp = "${data.day}-${Calendar.getInstance().get(Calendar.HOUR_OF_DAY)}"
            val key = "scrolled-$id"
            val countKey = "scroll-count-$id"
            val count = if (prefs.getString(key, null) == stamp) prefs.getInt(countKey, 0) else 0
            if (count >= SCROLLS_PER_HOUR) return null
            val renderer = rendererFor(context, data, id)
            val hour = renderer.allDayRows.size + renderer.scrollHour()
            // The list only scrolls until the row comes into view, so a row lands
            // on the bottom edge going down and on the top edge going up. Going
            // down, aim past the hour to leave it near the top.
            val going = hour + visibleHours(context, id) - 2
            val lastKey = "position-$id"
            val last = prefs.getInt(lastKey, 0)
            val target = if (going >= last) going else hour
            val position = target.coerceAtMost(renderer.allDayRows.size + 23)
            prefs.edit()
                .putString(key, stamp)
                .putInt(countKey, count + 1)
                .putInt(lastKey, position)
                .apply()
            return position
        }

        /** Whole hours that fit between the header and the footer. */
        private fun visibleHours(context: Context, id: Int): Int {
            val options = manager(context).getAppWidgetOptions(id)
            val landscape = context.resources.configuration.orientation ==
                Configuration.ORIENTATION_LANDSCAPE
            val height = options.getInt(
                if (landscape) {
                    AppWidgetManager.OPTION_APPWIDGET_MIN_HEIGHT
                } else {
                    AppWidgetManager.OPTION_APPWIDGET_MAX_HEIGHT
                },
            )
            return ((height - HEADER_AND_FOOTER_DP) / HOUR_HEIGHT_DP).toInt().coerceAtLeast(1)
        }

        private fun openAppIntent(context: Context, request: Int, flags: Int): PendingIntent {
            // The launcher's own intent, so this resumes the task the app is in.
            val launch = context.packageManager.getLaunchIntentForPackage(context.packageName)
                ?: Intent()
            return PendingIntent.getActivity(
                context,
                request,
                launch,
                flags or PendingIntent.FLAG_UPDATE_CURRENT,
            )
        }

        private fun tickIntent(context: Context): PendingIntent = PendingIntent.getBroadcast(
            context,
            0,
            Intent(context, DayWidgetProvider::class.java).setAction(ACTION_TICK),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )

        /** Moves the now line. Not a wakeup alarm: nobody sees it while the screen is off. */
        private fun scheduleTick(context: Context) {
            val now = System.currentTimeMillis()
            context.getSystemService(AlarmManager::class.java).set(
                AlarmManager.RTC,
                now - now % TICK_MILLIS + TICK_MILLIS,
                tickIntent(context),
            )
        }
    }
}
