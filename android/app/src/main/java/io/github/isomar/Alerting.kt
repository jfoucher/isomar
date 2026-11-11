package io.github.isomar

import android.Manifest
import android.annotation.SuppressLint
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.media.AudioAttributes
import android.media.MediaPlayer
import android.media.RingtoneManager
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat

/**
 * Alarms when the app is out of sight, the screen kept on while navigating, the app's sounds, and the
 * foreground service that keeps the engine (in the Application) running while something is watched: an
 * anchor watch, a route being followed, a track being recorded (`vm::window.watching`).
 */
object Alerting {
    const val CHANNEL_ALARMS = "alarms"
    const val CHANNEL_WATCH = "watch"
    const val CHANNEL_DONE = "done"
    private const val NOTIFY_ALARM = 2
    private const val NOTIFY_DONE = 3

    private lateinit var app: Context
    /** Set by the activity while it shows: the screen stays on. */
    var keepScreenOn: (Boolean) -> Unit = {}
    /** Set by the activity: it shows over the lock screen, and turns the screen on, while an alarm sounds. */
    var overLockScreen: (Boolean) -> Unit = {}
    private var overLock = false
    /** The activity is in the foreground (between onStart and onStop). */
    var visible = false
    private var notified: Long? = null
    private var watching = false

    fun init(context: Context) {
        app = context.applicationContext
        val nm = app.getSystemService(NotificationManager::class.java)
        val alarmSound = AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_ALARM).setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION).build()
        nm.createNotificationChannel(
            NotificationChannel(CHANNEL_ALARMS, "Alarms", NotificationManager.IMPORTANCE_HIGH).apply {
                description = "Anchor drag, collision risk, arrival and the other alarms"
                setSound(RingtoneManager.getDefaultUri(RingtoneManager.TYPE_ALARM), alarmSound)
                enableVibration(true)
                setShowBadge(true)
            },
        )
        nm.createNotificationChannel(
            NotificationChannel(CHANNEL_WATCH, "Watching", NotificationManager.IMPORTANCE_LOW).apply {
                description = "Shown while Isomar keeps watch with the screen off"
                setShowBadge(false)
            },
        )
        nm.createNotificationChannel(
            NotificationChannel(CHANNEL_DONE, "Calculations", NotificationManager.IMPORTANCE_DEFAULT).apply {
                description = "A weather route, best departure or ensemble check has finished"
            },
        )
    }

    /** A long calculation ended (`AppState::finished`; its sound has played): a notification while the app is out of sight. */
    fun finished(title: String, body: String) {
        if (visible || foreground() || screenshot) return
        if (Build.VERSION.SDK_INT >= 33 && app.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) return
        val open = PendingIntent.getActivity(app, 0, Intent(app, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP), PendingIntent.FLAG_IMMUTABLE)
        val n = NotificationCompat.Builder(app, CHANNEL_DONE)
            .setSmallIcon(android.R.drawable.ic_dialog_info)
            .setContentTitle(title)
            .setContentText(body)
            .setStyle(NotificationCompat.BigTextStyle().bigText(body))
            .setContentIntent(open)
            .setAutoCancel(true)
            .build()
        @SuppressLint("MissingPermission")
        app.getSystemService(NotificationManager::class.java).notify(NOTIFY_DONE, n)
    }

    fun update(alerts: AlertsInfo, window: WindowInfo) {
        keepScreenOn(window.underway && Model.keepOnUnderway)
        if (window.watching != watching) {
            watching = window.watching
            if (watching) WatchService.start(app) else WatchService.stop(app)
        }
        val alarm = alerts.alarm
        val sounding = alarm != null && !alarm.dim
        if (sounding != overLock) {
            overLock = sounding
            overLockScreen(sounding)
        }
        val nm = app.getSystemService(NotificationManager::class.java)
        if (alarm == null || alarm.dim) {
            if (notified != null) {
                notified = null
                nm.cancel(NOTIFY_ALARM)
            }
            return
        }
        if (visible || foreground() || screenshot || notified == alarm.id) return
        notified = alarm.id
        if (Build.VERSION.SDK_INT >= 33 && app.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) return
        val open = PendingIntent.getActivity(app, 0, Intent(app, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP), PendingIntent.FLAG_IMMUTABLE)
        val ack = PendingIntent.getBroadcast(
            app, 1, Intent(app, AlarmActions::class.java).putExtra("id", alarm.id),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val n = NotificationCompat.Builder(app, CHANNEL_ALARMS)
            .setSmallIcon(android.R.drawable.ic_dialog_alert)
            .setContentTitle("Isomar")
            .setContentText(alarm.title)
            .setCategory(NotificationCompat.CATEGORY_ALARM)
            .setPriority(NotificationCompat.PRIORITY_MAX)
            .setContentIntent(open)
            // Over the lock screen, as a phone call is, when the user allowed full-screen alarms.
            .setFullScreenIntent(open, true)
            .addAction(0, alarm.acknowledge ?: "Acknowledge", ack)
            .setOngoing(true)
            .build()
        @SuppressLint("MissingPermission")
        nm.notify(NOTIFY_ALARM, n)
    }

    /** Started for a screenshot: no notifications over it. */
    var screenshot = false

    /** The app is on screen, even before the activity said so (the first tick comes before it starts). */
    private fun foreground(): Boolean {
        val info = android.app.ActivityManager.RunningAppProcessInfo()
        android.app.ActivityManager.getMyMemoryState(info)
        return info.importance <= android.app.ActivityManager.RunningAppProcessInfo.IMPORTANCE_FOREGROUND
    }

    private val players = HashMap<String, MediaPlayer>()

    /** The app's sounds ship with it, so they work offline; the alarm plays as an alarm, through silent mode. */
    fun play(name: String) {
        val p = players.getOrPut(name) {
            MediaPlayer().apply {
                val usage = if (name == "alarm") AudioAttributes.USAGE_ALARM else AudioAttributes.USAGE_ASSISTANCE_SONIFICATION
                setAudioAttributes(AudioAttributes.Builder().setUsage(usage).setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION).build())
                val fd = runCatching { app.assets.openFd("sounds/$name.wav") }.getOrNull() ?: return
                fd.use { setDataSource(it.fileDescriptor, it.startOffset, it.length) }
                prepare()
            }
        }
        p.seekTo(0)
        p.start()
    }
}

/** The Acknowledge button of an alarm's notification. */
class AlarmActions : android.content.BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val id = intent.getLongExtra("id", 0)
        Native.command("alert_acknowledge", id)
    }
}

/**
 * Keeps the process, and so the engine, alive with the screen off while something is watched. Android
 * stops the GPS of an app out of sight unless it runs such a service of the location type.
 */
class WatchService : Service() {
    companion object {
        private const val NOTIFY_WATCH = 1

        fun start(context: Context) {
            val i = Intent(context, WatchService::class.java)
            runCatching { context.startForegroundService(i) }
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, WatchService::class.java))
        }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)
        val n: Notification = NotificationCompat.Builder(this, Alerting.CHANNEL_WATCH)
            .setSmallIcon(android.R.drawable.ic_menu_mylocation)
            .setContentTitle("Isomar is keeping watch")
            .setContentText("Alarms sound with the screen off")
            .setContentIntent(open)
            .setOngoing(true)
            .build()
        val type = if (Build.VERSION.SDK_INT >= 29) ServiceInfo.FOREGROUND_SERVICE_TYPE_LOCATION else 0
        val started = runCatching { ServiceCompat.startForeground(this, NOTIFY_WATCH, n, type) }.isSuccess
        if (!started) stopSelf()
        return START_STICKY
    }
}
