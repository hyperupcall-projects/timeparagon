package dev.kofler.timeparagon.widget

import android.appwidget.AppWidgetManager
import android.content.Context
import android.content.Intent
import android.widget.RemoteViews
import android.widget.RemoteViewsService
import dev.kofler.timeparagon.R

/** Supplies the widget's list: all-day chips first, then one row per hour. */
class DayWidgetService : RemoteViewsService() {
    override fun onGetViewFactory(intent: Intent): RemoteViewsFactory =
        Factory(
            applicationContext,
            intent.getIntExtra(
                AppWidgetManager.EXTRA_APPWIDGET_ID,
                AppWidgetManager.INVALID_APPWIDGET_ID,
            ),
        )

    private class Factory(
        private val context: Context,
        private val widgetId: Int,
    ) : RemoteViewsFactory {
        private var renderer: DayRenderer? = null
        private var allDayRows: List<List<AllDayEvent>> = emptyList()

        override fun onCreate() = Unit

        override fun onDataSetChanged() {
            val data = WidgetDay.load(context)
            renderer = data?.let {
                DayWidgetProvider.rendererFor(context, it, widgetId)
            }
            allDayRows = renderer?.allDayRows.orEmpty()
        }

        override fun onDestroy() = Unit

        override fun getCount(): Int = if (renderer == null) 0 else allDayRows.size + 24

        override fun getViewTypeCount(): Int = 2

        override fun getItemId(position: Int): Long = position.toLong()

        override fun hasStableIds(): Boolean = false

        override fun getLoadingView(): RemoteViews =
            RemoteViews(context.packageName, R.layout.day_widget_loading_row)

        override fun getViewAt(position: Int): RemoteViews {
            val renderer = renderer ?: return getLoadingView()
            val allDay = position < allDayRows.size
            val views = RemoteViews(
                context.packageName,
                if (allDay) R.layout.day_widget_all_day_row else R.layout.day_widget_hour_row,
            )
            val bitmap = if (allDay) {
                renderer.drawAllDayRow(allDayRows[position])
            } else {
                renderer.drawHour(position - allDayRows.size)
            }
            views.setImageViewBitmap(R.id.day_widget_row_image, bitmap)
            views.setOnClickFillInIntent(R.id.day_widget_row_image, Intent())
            return views
        }
    }
}
