package io.github.isomar

import android.app.Application
import android.os.Handler
import android.os.Looper
import android.system.Os
import java.io.File
import java.util.Locale

/**
 * Owns the engine, so that it outlives the Activity (a fold, a rotation, the screen going off).
 *
 * Before the engine starts it is told where everything lives (Android has no home folder for the Rust
 * side to start from), and fontconfig is given a configuration: Android has none, and pango finds its
 * fonts through it. The fonts are Adwaita Sans, copied from the assets, then the system's, so that names in
 * any script find a font that has their characters.
 */
class IsomarApp : Application() {
    companion object {
        /** hit-min-android (data/design/tokens.json): 48 dp. */
        const val TOUCH_HIT_MIN = 48.0
    }

    private val main = Handler(Looper.getMainLooper())

    override fun onCreate() {
        super.onCreate()
        val files = filesDir
        val cache = cacheDir
        fun dir(parent: File, name: String) = File(parent, name).apply { mkdirs() }.absolutePath
        Os.setenv("HOME", files.absolutePath, true)
        Os.setenv("TMPDIR", cache.absolutePath, true)
        Os.setenv("ISOMAR_CONFIG_DIR", dir(files, "config"), true)
        Os.setenv("ISOMAR_STATE_DIR", dir(files, "state"), true)
        Os.setenv("ISOMAR_DATA_DIR", dir(files, "data"), true)
        Os.setenv("ISOMAR_CACHE_DIR", dir(cache, "isomar"), true)
        Os.setenv("LANGUAGE", Locale.getDefault().toLanguageTag().replace('-', '_'), true)
        Os.setenv("FONTCONFIG_FILE", writeFontConfig().absolutePath, true)
        debugEnv()

        Native.start()
        Alerting.init(this)
        UsbNmea.init(this)
        Model.start()
        Native.setReduceMotion(android.provider.Settings.Global.getFloat(contentResolver, android.provider.Settings.Global.ANIMATOR_DURATION_SCALE, 1f) == 0f)
        Native.setTouch(TOUCH_HIT_MIN)
        // Native memory is not the Java heap, but the low-memory killer counts it: a smaller tile cache than
        // a desktop's 192 MB, by how much memory the device has.
        val mem = android.app.ActivityManager.MemoryInfo()
        (getSystemService(ACTIVITY_SERVICE) as android.app.ActivityManager).getMemoryInfo(mem)
        Native.setTileCacheLimit(if (mem.totalMem <= 4L shl 30) 64L shl 20 else 96L shl 20)
        Model.onConnections = ::multicast
        multicast()
        tick()
    }

    private var multicastLock: android.net.wifi.WifiManager.MulticastLock? = null

    /** Android drops broadcast and multicast packets to save power unless an app holds this lock: held while a UDP connection listens. */
    private fun multicast() {
        val want = Model.listensUdp()
        val held = multicastLock?.isHeld == true
        if (want && !held) {
            val wifi = applicationContext.getSystemService(WIFI_SERVICE) as android.net.wifi.WifiManager
            multicastLock = wifi.createMulticastLock("isomar-nmea").apply { setReferenceCounted(false); acquire() }
        } else if (!want && held) {
            multicastLock?.release()
        }
    }

    override fun onTrimMemory(level: Int) {
        super.onTrimMemory(level)
        // In the background or short of memory: keep what the view needs, drop the rest.
        if (level >= TRIM_MEMORY_BACKGROUND) Native.setTileCacheLimit(32L shl 20)
    }

    private fun tick() {
        Native.tick()
        main.postDelayed(::tick, 250)
    }

    /**
     * A debuggable build reads `files/debug.env` (KEY=VALUE lines) before the engine starts: the engine's
     * ISOMAR_DEMO, ISOMAR_DEBUG_VIEW and the like, which Android gives no other way to set.
     * `adb shell run-as eu.sixpx.isomar sh -c 'echo ISOMAR_DEMO=1 > files/debug.env'`
     */
    private fun debugEnv() {
        if (applicationInfo.flags and android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE == 0) return
        val f = File(filesDir, "debug.env")
        if (!f.exists()) return
        for (line in f.readLines()) {
            val (k, v) = line.split('=', limit = 2).takeIf { it.size == 2 } ?: continue
            Os.setenv(k.trim(), v.trim(), true)
        }
    }

    private fun writeFontConfig(): File {
        val fonts = File(filesDir, "fonts").apply { mkdirs() }
        for (name in assets.list("fonts").orEmpty()) {
            val out = File(fonts, name)
            if (!out.exists()) assets.open("fonts/$name").use { input -> out.outputStream().use { input.copyTo(it) } }
        }
        val cache = File(cacheDir, "fontconfig").apply { mkdirs() }
        val conf = File(filesDir, "fonts.conf")
        conf.writeText(
            """
            <?xml version="1.0"?>
            <!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
            <fontconfig>
              <dir>${fonts.absolutePath}</dir>
              <dir>/system/fonts</dir>
              <dir>/product/fonts</dir>
              <cachedir>${cache.absolutePath}</cachedir>
              <alias>
                <family>sans-serif</family>
                <prefer><family>Adwaita Sans</family><family>Roboto</family><family>Noto Sans</family></prefer>
              </alias>
            </fontconfig>
            """.trimIndent(),
        )
        return conf
    }
}
